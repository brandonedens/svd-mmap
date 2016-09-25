// Copyright 2016 by the svd-mmap project developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//
// Author: Brandon Edens <brandonedens@gmail.com>
// Date: 2016-08-15

//! Provide a Rust macro for converting ARM CMSIS SVD description into Rust for accessing the
//! specified hardware.

#![feature(plugin, plugin_registrar, rustc_private)]
#![plugin(quasi_macros)]

extern crate aster;
extern crate inflections;
extern crate quasi;
extern crate rustc;
extern crate rustc_plugin;
extern crate svd;
extern crate syntax;

use inflections::Inflect;
use rustc_plugin::Registry;
use std::borrow::Borrow;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::prelude::*;
use svd::{Access, Device, Field, Peripheral, Register};
use syntax::ast;
use syntax::codemap::Span;
use syntax::ext::base::{DummyResult, ExtCtxt, MacResult};
use syntax::parse::token;
use syntax::ptr::P;
use syntax::tokenstream;
use syntax::util::small_vector::SmallVector;

const LINK_MEM_PREFIX: &'static str = "mmap_";

trait GenField {
    /// Generate getter impl.
    fn gen_get(&self, cx: &ExtCtxt, register: &Register) -> Vec<P<syntax::ast::Item>>;

    /// Generate type of the field.
    fn gen_type(&self) -> syntax::ptr::P<syntax::ast::Ty>;

    /// Generate the type definition for the field that has enumerated values.
    fn gen_type_def(&self, cx: &ExtCtxt) -> Option<P<syntax::ast::Item>>;

    /// Generate setter impl.
    fn gen_update(&self, cx: &ExtCtxt, register: &Register) -> Vec<P<syntax::ast::Item>>;
}

impl GenField for Field {

    /// Generate struct representation of register field getter in the form:
    ///
    /// ```rust
    /// #[allow(dead_code, missing_docs)]
    /// impl Cr {
    ///     #[inline(always)]
    ///     pub fn rx(&self) -> bool {
    ///         CrGet::new(self).rx()
    ///     }
    /// }
    ///
    /// #[allow(dead_code, missing_docs)]
    /// impl CrGet {
    ///     #[inline(always)]
    ///     pub fn rx(&self) -> CrGet {
    ///         (self.value >> 11) & 1 != 0
    ///     }
    /// }
    /// ```
    fn gen_get(&self, cx: &ExtCtxt, register: &Register) -> Vec<P<syntax::ast::Item>> {
        let builder    = aster::AstBuilder::new();
        let field_name = builder.id(self.name.to_snake_case());
        let field_ty   = self.gen_type();
        let bit_offset = self.bit_range.offset;
        let bit_width  = self.bit_range.width;

        let reg_name_get = register.getter_name();
        let reg_type_name = register.type_name();

        let mut v = Vec::new();
        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        impl $reg_type_name {
                            #[inline(always)]
                            pub fn $field_name(&self) -> $field_ty {
                                $reg_name_get::new(self).$field_name()
                            }
                        }).unwrap());

        v.push(
            if let Some(enum_vals) = self.enumerated_values.as_ref() {
                let keys = enum_vals.values.iter()
                    .map(|x| builder.id(x.name.to_pascal_case()))
                    .collect::<Vec<_>>().into_iter();
                let vals = enum_vals.values.iter()
                    .map(|x| x.value)
                    .collect::<Vec<_>>().into_iter();

                let ref name = enum_vals.name.as_ref().unwrap_or(&self.name);
                let enum_name = builder.id(name.to_pascal_case());

                quote_item!(&cx,
                            #[allow(dead_code, missing_docs)]
                            impl $reg_name_get {
                                #[inline(always)]
                                pub fn $field_name(&self) -> $field_ty {
                                    match (self.value >> $bit_offset) & $bit_width {
                                        $($vals => ::core::option::Option::Some($enum_name::$keys)),*,
                                        _ => ::core::option::Option::None,
                                    }.unwrap()
                                }
                            }).unwrap()

            } else if self.bit_range.width == 1 {
                quote_item!(&cx,
                            #[allow(dead_code, missing_docs)]
                            impl $reg_name_get {
                                #[inline(always)]
                                pub fn $field_name(&self) -> $field_ty {
                                    (self.value >> $bit_offset) & $bit_width != 0
                                }
                            }).unwrap()

            } else {
                quote_item!(&cx,
                            #[allow(dead_code, missing_docs)]
                            impl $reg_name_get {
                                #[inline(always)]
                                pub fn $field_name(&self) -> $field_ty {
                                    ((self.value >> $bit_offset) & $bit_width) as $field_ty
                                }
                            }).unwrap()
            });
        v
    }

    /// Generate a type for this field.
    ///
    /// A type could be bool, u8, u16, or some enum like Parity depending upon the bit width and
    /// potential existence of enumerated values.
    fn gen_type(&self) -> syntax::ptr::P<syntax::ast::Ty> {
        let builder = aster::AstBuilder::new();

        if let Some(vals) = self.enumerated_values.as_ref() {
            let ref name = vals.name.as_ref().unwrap_or(&self.name);
            builder.ty().id(name.to_pascal_case())
        } else {
            match self.bit_range.width {
                1 => builder.ty().bool(),
                2...8 => builder.ty().u8(),
                9...16 => builder.ty().u16(),
                17...32 => builder.ty().u32(),
                33...64 => builder.ty().u64(),
                _ => panic!("Unknown bit width"),
            }
        }
    }

    /// Generate a type for this field if applicable in the form of:
    ///
    /// ```rust
	/// #[derive(PartialEq)]
	/// #[allow(dead_code, missing_docs)]
	/// #[repr(u32)]
	/// pub enum Parity {
	///     None = 0,
	///     Even = 2,
	///     Odd = 3,
	/// }
    /// ```
    fn gen_type_def(&self, cx: &ExtCtxt) -> Option<P<syntax::ast::Item>> {
		if self.enumerated_values.is_none() {
            return None;
        }

        let builder = aster::AstBuilder::new();
        let enum_vals = self.enumerated_values.as_ref().unwrap();
        let ref name = enum_vals.name.as_ref().unwrap_or(&self.name);
        let name = builder.id(name.to_pascal_case());

        let keys = enum_vals.values.iter()
            .map(|x| builder.id(x.name.to_pascal_case()))
            .collect::<Vec<_>>().into_iter();
        let vals = enum_vals.values.iter()
            .map(|x| x.value)
            .collect::<Vec<_>>().into_iter();

        Some(quote_item!(&cx,
                         #[derive(PartialEq)]
                         #[allow(dead_code, missing_docs)]
                         #[repr(u32)]
                         pub enum $name {
                             $($keys = $vals),*
                         }).unwrap())
    }

    /// Generate struct representation of register field update in the form of:
    ///
    /// ```rust
    /// impl Cr {
    ///     #[inline(always)]
    ///     pub fn set_rx<'a>('a self, new_value: bool) -> CrUpdate<'a> {
    ///         let mut setter: CrUpdate = CrUpdate::new(self);
    ///         setter.set_rx(new_value);
    ///         setter
    ///     }
    /// }
    ///
    /// impl<'a> CrUpdate<'a> {
    ///     #[inline(always)]
    ///     pub fn set_rx<'b>(&'b mut self, new_value: bool) -> &'b mut CrUpdate<'a> {
    ///         self.value = (self.value & !(1 << 11)) |
    ///             ((new_value as u32) & 1) << 11;
    ///         self.mask |= 1 << 11;
    ///         self
    ///     }
    /// }
    /// ```
    fn gen_update(&self, cx: &ExtCtxt, register: &Register) -> Vec<P<syntax::ast::Item>> {
        let builder    = aster::AstBuilder::new();
        let field_name = builder.id("set_".to_string() + &self.name.to_snake_case());
        let field_ty   = self.gen_type();
        let bit_offset = self.bit_range.offset;
        let bit_width  = self.bit_range.width;

        let reg_name_update = register.updater_name();
        let reg_type_name = register.type_name();

        let mut v = Vec::new();
        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        impl $reg_type_name {
                            #[inline(always)]
                            pub fn $field_name<'a>(&'a self, new_value: $field_ty) -> $reg_name_update<'a> {
                                let mut setter: $reg_name_update = $reg_name_update::new(self);
                                setter.$field_name(new_value);
                                setter
                            }
                        }
                       ).unwrap());

        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        impl<'a> $reg_name_update<'a> {
                            #[inline(always)]
                            pub fn $field_name<'b>(&'b mut self, new_value: $field_ty) -> &'b mut $reg_name_update<'a> {
                                self.value = (self.value & !($bit_width << $bit_offset)) |
                                    ((new_value as u32) & $bit_width) << $bit_offset;
                                self.mask |= $bit_width << $bit_offset;
                                self
                            }
                        }).unwrap());
        v
    }
}

trait GenReg {
    /// Generate register memory map information (including fields).
    fn gen_mmap(&self, cx: &ExtCtxt) -> Vec<P<syntax::ast::Item>>;

    /// Generate register constants information.
    fn gen_const(&self, cx: &ExtCtxt) -> Vec<P<syntax::ast::Item>>;

    // Generate getter information.
    fn gen_getter(&self, cx: &ExtCtxt) -> Vec<P<syntax::ast::Item>>;

    // Generate updater information.
    fn gen_updater(&self, cx: &ExtCtxt) -> Vec<P<syntax::ast::Item>>;

    /// Generate getter name.
    fn getter_name(&self) -> ast::Ident;

    /// Generate type name.
    fn type_name(&self) -> ast::Ident;

    /// Generate updater name.
    fn updater_name(&self) -> ast::Ident;

}

impl GenReg for Register {

    /// Generate all of the Rust code needed to interface to this regster.
    fn gen_mmap(&self, cx: &ExtCtxt) -> Vec<P<syntax::ast::Item>> {
        let mut v = Vec::new();

        // First we generate constant software associated with all registers.
        v.append(&mut self.gen_const(&cx));

        if self.access != Some(Access::WriteOnly) {
            // Now we generate Get specific software.
            v.append(&mut self.gen_getter(&cx));
        }

        if self.access != Some(Access::ReadOnly) {
            // Now we generate Update specific software.
            v.append(&mut self.gen_updater(&cx));
        }

        // Begin generating field information.
        if let Some(fields) = self.fields.as_ref() {
            // Generate the field's type definitions if necessary.
            v.append(&mut fields.iter()
                     .filter_map(|x| x.gen_type_def(&cx))
                     .collect::<Vec<_>>());

            if self.access != Some(Access::WriteOnly) {
                // For each of the register's fields we generate the field's getter.
                v.append(&mut
                         fields.iter()
                         .filter(|x| x.access != Some(Access::WriteOnly))
                         .flat_map(|x| x.gen_get(&cx, self))
                         .collect::<Vec<_>>());
            }

            if self.access != Some(Access::ReadOnly) {
                // and updater.
                v.append(&mut
                         fields.iter()
                         .filter(|x| x.access != Some(Access::ReadOnly))
                         .flat_map(|x| x.gen_update(&cx, self))
                         .collect::<Vec<_>>());
            }
        }
        v
    }

    /// Generate all of the constant register details.
    ///
    /// The result should look like:
    ///
    /// ```rust
    /// #[allow(dead_code), missing_docs)]
    /// #[repr(C)]
    /// pub struct Cr {
    ///     value: VolatileCell<u32>,
    /// }
    /// ```
    fn gen_const(&self, cx: &ExtCtxt) -> Vec<P<syntax::ast::Item>> {
        let mut v = Vec::new();

        let reg_type_name = self.type_name();

        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        #[repr(C)]
                        pub struct $reg_type_name {
                            value: VolatileCell<u32>,
                        }).unwrap());

        v
    }

    /// Generate all of the constant register details for getters.
    ///
    /// The result should look like:
    ///
    /// ```rust
    /// #[allow(dead_code), missing_docs)]
    /// impl Cr {
    ///     #[inline(always)]
    ///     pub fn get(&self) -> CrGet {
    ///         CrGet::new(self)
    ///     }
    /// }
    ///
    /// #[allow(dead_code), missing_docs)]
    /// #[derive(Clone)]
    /// pub struct CrGet {
    ///     value: u32,
    /// }
    ///
    /// #[allow(dead_code), missing_docs)]
    /// #[derive(Clone)]
    /// impl CrGet {
    ///     #[inline(always)]
    ///     pub fn new(reg: Cr) -> CrGet {
    ///         CrGet { value: reg.value.get() }
    ///     }
    /// }
    /// ```
    fn gen_getter(&self, cx: &ExtCtxt) -> Vec<P<syntax::ast::Item>> {
        let mut v = Vec::new();
        let reg_type_name = self.type_name();
        let reg_name_get = self.getter_name();

        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        #[derive(Clone)]
                        pub struct $reg_name_get {
                            value: u32,
                        }).unwrap());

        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        impl $reg_type_name {
                            #[inline(always)]
                            pub fn get(&self) -> $reg_name_get {
                                $reg_name_get::new(self)
                            }
                        }).unwrap());

        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        impl $reg_name_get {
                            #[inline(always)]
                            pub fn new(reg: &$reg_type_name) -> $reg_name_get {
                                $reg_name_get { value: reg.value.get() }
                            }
                        }).unwrap());
        v
    }

    /// Generate all of the constant register details for getters.
    ///
    /// The result should look like:
    ///
    /// ```rust
    /// #[allow(dead_code), missing_docs)]
    /// impl Cr {
    ///     #[inline(always)]
    ///     pub fn ignoring_state(&self) -> CrUpdate {
    ///         CrUpdate::new_ignoring_state(self)
    ///     }
    /// }
    ///
    /// #[allow(dead_code), missing_docs)]
    /// pub struct CrUpdate<'a> {
    ///     value: u32,
    ///     mask: u32,
    ///     write_only: bool,
    ///     reg: &'a Cr,
    /// }
    ///
    /// TODO is the clear mask correct?
    /// #[allow(dead_code), missing_docs)]
    /// impl<'a> Drop for CrUpdate<'a> {
    ///     #[inline(always)]
    ///     fn drop(&mut self) {
    ///         let clear_mask: u32 = 1u32 as u32;
    ///         if self.mask != 0 {
    ///             let v: u32 =
    ///                 if self.write_only { 0 } else { self.reg.value.get() } &
    ///                     !clear_mask & !self.mask;
    ///             self.reg.value.set(self.value | v);
    ///         }
    ///     }
    /// }
    ///
    /// #[allow(dead_code), missing_docs)]
    /// impl<'a> CrUpdate<'a> {
    ///     #[inline(always)]
    ///     pub fn new(reg: &'a Cr) -> CrUpdate<'a> {
    ///         CrUpdate { value: 0, mask: 0, write_only: false, reg: reg }
    ///     }
    ///     #[inline(always)]
    ///     pub fn new_ignoring_state(reg: &'a Cr) -> CrUpdate<'a> {
    ///         CrUpdate { value: 0, mask: 0, write_only: true, reg: reg }
    ///     }
    /// }
    /// ```
    fn gen_updater(&self, cx: &ExtCtxt) -> Vec<P<syntax::ast::Item>> {
        let mut v = Vec::new();
        let reg_type_name = self.type_name();
        let reg_name_update = self.updater_name();

        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        pub struct $reg_name_update<'a> {
                            value: u32,
                            mask: u32,
                            write_only: bool,
                            reg: &'a $reg_type_name,
                        }).unwrap());

        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        impl $reg_type_name {
                            #[inline(always)]
                            pub fn ignoring_state(&self) -> $reg_name_update {
                                $reg_name_update::new_ignoring_state(self)
                            }
                        }).unwrap());


        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        impl<'a> Drop for $reg_name_update<'a> {
                            #[inline(always)]
                            fn drop(&mut self) {
                                let clear_mask: u32 = 1u32 as u32;
                                if self.mask != 0 {
                                    let v: u32 =
                                        if self.write_only { 0 } else { self.reg.value.get() } &
                                            !clear_mask & !self.mask;
                                    self.reg.value.set(self.value | v);
                                }
                            }
                        }).unwrap());

        v.push(
            quote_item!(&cx,
                        #[allow(dead_code, missing_docs)]
                        impl<'a> $reg_name_update<'a> {
                            #[inline(always)]
                            pub fn new(reg: &'a $reg_type_name) -> $reg_name_update<'a> {
                                $reg_name_update {value: 0, mask: 0, write_only: false, reg: reg}
                            }

                            #[inline(always)]
                            pub fn new_ignoring_state(reg: &'a $reg_type_name) -> $reg_name_update<'a> {
                                $reg_name_update {value: 0, mask: 0, write_only: true, reg: reg}
                            }
                        }).unwrap());
        v
    }

    /// Generate getter name.
    fn getter_name(&self) -> ast::Ident {
        let builder = aster::AstBuilder::new();
        let name = self.name.to_pascal_case();
        builder.id(name.to_owned() + "Get")
    }

    /// Generate type name.
    fn type_name(&self) -> ast::Ident {
        let builder = aster::AstBuilder::new();
        let name = self.name.to_pascal_case();
        builder.id(name.to_owned())
    }

    /// Generate updater name.
    fn updater_name(&self) -> ast::Ident {
        let builder = aster::AstBuilder::new();
        let name = self.name.to_pascal_case();
        builder.id(name.to_owned() + "Update")
    }
}

/// Generate complete memory mapped hardware definition in Rust for device.
pub fn gen_device(cx: &mut ExtCtxt, device: &Device) -> Vec<P<syntax::ast::Item>> {
    let builder = aster::AstBuilder::new();

    // First find all peripherals that have other peripherals derived from them.
    let mut derived_from: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for ref periph in device.peripherals.iter() {
        // Iterate through the peripherals and add each derived_from name to the map of name to a
        // set of dependent peripherals.
        if let Some(derived_name) = periph.derived_from.as_ref() {
            derived_from.insert(derived_name, BTreeSet::new());
        }
    }
    for ref periph in device.peripherals.iter() {
        // Iterate through the peripherals and assign the peripheral that is derived from to each
        // set.
        if let Some(derived_name) = periph.derived_from.as_ref() {
            let set = derived_from.get_mut::<&str>(&derived_name.as_str()).unwrap();
            set.insert(&periph.name);
        }
    }

    // Set of module names already defined.
    let mut module_name_set: BTreeSet<&str> = BTreeSet::new();

    let mut peripheral_items = Vec::new();
    for periph in device.peripherals.iter() {

        if periph.derived_from.is_none() {
            let periph_items = gen_periph(cx, periph);

            // Wrap the peripheral items in a module.
            let name = periph.name.as_str();
            let group_name = periph.group_name.as_ref();

            let periph_name = if derived_from.contains_key(name) &&
                group_name.is_some() &&
                !module_name_set.contains(group_name.unwrap().as_str()) {
                    group_name.unwrap()
                } else {
                    name
                };
            let periph_mod_name = builder.id(periph_name.to_snake_case());
            module_name_set.insert(periph_name);

            // Build the variables that represent access to the hardware.
            let link_name = String::from(LINK_MEM_PREFIX.to_owned() + &device.name + "_" + &periph.name).to_snake_case();
            let periph_ty = builder.id(
                periph.group_name.as_ref().unwrap_or(&periph_name.to_owned()).to_pascal_case());
            let periph_name = builder.id(periph.name.to_constant_case());

            // Build the links to memory mapped registers.
            let mut statics = Vec::new();
            let item = quote_item!(&cx,
                                   #[allow(dead_code)]
                                   extern {
                                       #[link_name=$link_name]
                                       pub static $periph_name: $periph_ty;
                                   }).unwrap();
            statics.push(item);
            if let Some(set) = derived_from.get(&periph.name.borrow()) {
                for periph_name in set {
                    let link_name =
                        String::from(LINK_MEM_PREFIX.to_owned() +
                                     &device.name + "_" +
                                     periph_name).to_snake_case();
                    let periph_name = builder.id(periph_name.to_constant_case());
                    let item = quote_item!(&cx,
                                           #[allow(dead_code)]
                                           extern {
                                               #[link_name=$link_name]
                                               pub static $periph_name: $periph_ty;
                                           }).unwrap();
                    statics.push(item);
                }
            }

            let periph_item = quote_item!(&cx, pub mod $periph_mod_name {
                use volatile_cell::VolatileCell;
                use core::ops::Drop;

                $periph_items
                $statics
            }).unwrap();

            //v.append(&mut gen_periph(cx, periph));
            peripheral_items.push(periph_item);
        }
    }

    // Create module housing the hardware.
    let dev_name =  builder.id(device.name.to_snake_case());
    let dev_item = quote_item!(&cx, pub mod $dev_name {
        $peripheral_items
    }).unwrap();

    let mut v = Vec::new();
    v.push(dev_item);
    v
}

/// Print to standard output linker information for the device.
pub fn gen_link_mem(device: &Device) {
    for periph in device.peripherals.iter() {
        let name = String::from(LINK_MEM_PREFIX.to_owned() +
                                &device.name + "_" +
                                periph.name.as_str()).to_snake_case();
        println!("{} = 0x{:08x}", name, periph.base_address);
    }
}

/// Generate definition of a peripheral.
fn gen_periph(cx: &ExtCtxt, periph: &Peripheral) -> Vec<P<syntax::ast::Item>> {
    let mut v = Vec::new();
    let builder = aster::AstBuilder::new();

    let periph_name = builder.id(
        periph.group_name.as_ref().unwrap_or(&periph.name).to_pascal_case());

    // Construct the vector of registers.
    let mut reg_vec = Vec::new();
    if let Some(regs) = periph.registers.as_ref() {
        // Sort the registers by their address offset before adding them to the struct represented
        // in C style.
        let mut sorted_regs: Vec<&Register> = regs.iter().map(|x| x).collect();
        sorted_regs.sort_by_key(|r| r.address_offset);
        let mut offset = 0u32;
        let mut pad_num = 0;
        for reg in sorted_regs {
            if reg.address_offset < offset {
                // We have registers that are located at same memory address as other registers.
                // Let's just ignore them.
                // FIXME should we handle this better?
                continue;

            } else if offset != reg.address_offset {
                // We need to introduce padding into the struct.
                let pad_name = builder.id(format!("_pad{}", pad_num));
                pad_num += 1;

                let delta = (reg.address_offset - offset) as usize;
                let tts = quote_tokens!(&cx, $pad_name: [u8; $delta],);
                reg_vec.push(tts);
            }

            let reg_var_name = builder.id(reg.name.to_snake_case());
            let reg_ty_name = builder.id(reg.name.to_pascal_case());
            let tts = quote_tokens!(&cx, pub $reg_var_name: $reg_ty_name,);
            reg_vec.push(tts);

            offset = reg.address_offset + 4;
        }
    }

    let item = quote_item!(&cx,
                           #[allow(dead_code, missing_docs)]
                           #[repr(C)]
                           pub struct $periph_name
                           {
                               $reg_vec
                           }).unwrap();
    v.push(item);

    if let Some(regs) = periph.registers.as_ref() {
        for reg in regs {
            v.append(&mut reg.gen_mmap(cx));
        }
    }

    v
}

#[plugin_registrar]
pub fn plugin_registrar(reg: &mut Registry) {
    reg.register_macro("svd_mmap", macro_svd_mmap);
}

pub struct MacItems {
    items: Vec<P<ast::Item>>,
}

impl MacItems {
    pub fn new(items: Vec<P<ast::Item>>) -> Box<MacResult + 'static> {
        Box::new(MacItems { items: items })
    }
}

impl MacResult for MacItems {
    fn make_items(self: Box<MacItems>) -> Option<SmallVector<P<ast::Item>>> {
        Some(SmallVector::many(self.items.clone()))
    }
}

pub fn macro_svd_mmap(cx: &mut ExtCtxt,
                      sp: Span,
                      tts: &[tokenstream::TokenTree])
                      -> Box<MacResult + 'static> {
    let mut v = std::vec::Vec::new();

    if tts.len() != 1 {
        cx.span_err(sp, &format!("argument must be single filename, but got {}",
                                 tts.len()));
        return DummyResult::any(sp);
    }

    let filename = match tts[0] {
        tokenstream::TokenTree::Token(_, token::Literal(token::Lit::Str_(s), _)) => s.to_string(),
        _ => {
            cx.span_err(sp, "argument must be filename, but got {}",);
            return DummyResult::any(sp);
        }
    };

    let mut svd_file = File::open(filename).unwrap();
    let mut s = String::new();
    svd_file.read_to_string(&mut s).unwrap();

    // Generate SVD device data from SVD XML.
    let dev = Device::parse(&s);

    v.append(&mut gen_device(cx, &dev));

    // TODO generate source code from SVD file given.
    MacItems::new(v)
}

#[cfg(test)]
mod tests {

    use aster::AstBuilder;
    use aster::name::ToName;
    use std::fs::File;
    use std::io::prelude::*;
    use svd::{Access, BitRange, Device, EnumeratedValue, EnumeratedValues, Field, Peripheral, Register};
    use syntax::codemap;
    use syntax::ext::base::{DummyResolver, ExtCtxt};
    use syntax::ext::expand;
    use syntax::parse;
    use syntax::print::pprust::item_to_string;
    use super::{GenField, GenReg};

    fn make_ext_ctxt<'a>(sess: &'a parse::ParseSess,
                         macro_loader: &'a mut DummyResolver) -> ExtCtxt<'a> {
        let info = codemap::ExpnInfo {
            call_site: codemap::DUMMY_SP,
            callee: codemap::NameAndSpan {
                format: codemap::MacroAttribute("test".to_name()),
                allow_internal_unstable: false,
                span: None
            }
        };

        let cfg = Vec::new();
        let ecfg = expand::ExpansionConfig::default(String::new());

        let mut cx = ExtCtxt::new(&sess, cfg, ecfg, macro_loader);
        cx.bt_push(info);

        cx
    }

    #[test]
    fn test_svd_gen_to_stdout() {
        let svd_filename = "/tmp/STM32L4x6.svd";
        let mut svd_file = File::open(svd_filename).unwrap();
        let mut s = String::new();
        svd_file.read_to_string(&mut s).unwrap();

        // Generate SVD device data from SVD XML.
        let dev = Device::parse(&s);

        let sess = parse::ParseSess::new();
        let mut macro_loader = DummyResolver;
        let mut cx = make_ext_ctxt(&sess, &mut macro_loader);

        let items = super::gen_device(&mut cx, &dev);
        for item in items {
            println!("{}", item_to_string(&item));
        }
    }

    #[test]
    fn test_gen_link_mem() {
        let svd_filename = "/tmp/STM32L4x6.svd";
        let mut svd_file = File::open(svd_filename).unwrap();
        let mut s = String::new();
        svd_file.read_to_string(&mut s).unwrap();

        // Generate SVD device data from SVD XML.
        let dev = Device::parse(&s);
        super::gen_link_mem(&dev);
    }

    #[test]
    fn test_gen_periph() {
        let spe = Field {
            name: "SPE".to_owned(),
            description: Some("SPI enable".to_owned()),
            bit_range: BitRange {
                offset: 6,
                width: 1,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: None,
        };
        let txe = Field {
            name: "TXE".to_owned(),
            description: Some("Transmit enable".to_owned()),
            bit_range: BitRange {
                offset: 7,
                width: 1,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: None,
        };
        let cr = Register {
            name: "CR".to_owned(),
            description: "Control register".to_owned(),
            fields: Some(vec![spe, txe]),
            access: None,
            size: Some(32),
            reset_mask: None,
            reset_value: None,
            address_offset: 0x00000000,
        };

        let foo = Field {
            name: "FOO".to_owned(),
            description: Some("Foo enable".to_owned()),
            bit_range: BitRange {
                offset: 0,
                width: 1,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: None,
        };
        let bar = Field {
            name: "BAR".to_owned(),
            description: Some("Bar enable".to_owned()),
            bit_range: BitRange {
                offset: 1,
                width: 1,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: None,
        };
        let baz = Register {
            name: "BAZ".to_owned(),
            description: "Baz register".to_owned(),
            fields: Some(vec![foo, bar]),
            access: None,
            size: Some(32),
            reset_mask: None,
            reset_value: None,
            address_offset: 0x00000004,
        };

        let periph = Peripheral {
            name: "Test2".to_owned(),
            group_name: Some("Test".to_owned()),
            description: None,
            base_address: 0xE000E000,
            interrupt: None,
            registers: Some(vec![cr, baz]),
            derived_from: None,
        };

        let sess = parse::ParseSess::new();
        let mut macro_loader = DummyResolver;
        let cx = make_ext_ctxt(&sess, &mut macro_loader);

        let items = super::gen_periph(&cx, &periph);
        for item in items {
            println!("{}", item_to_string(&item));
        }
    }

    #[test]
    fn test_gen_reg_field_impl() {
        let spe = Field {
            name: "SPE".to_owned(),
            description: Some("SPI enable".to_owned()),
            bit_range: BitRange {
                offset: 6,
                width: 1,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: None,
        };

        let txe = Field {
            name: "TXE".to_owned(),
            description: Some("Transmit enable".to_owned()),
            bit_range: BitRange {
                offset: 7,
                width: 1,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: None,
        };

        let freq = Field {
            name: "FREQ".to_owned(),
            description: Some("Frequency".to_owned()),
            bit_range: BitRange {
                offset: 8,
                width: 4,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: None,
        };

        let reg = Register {
            name: "CR".to_owned(),
            description: "Control register".to_owned(),
            fields: Some(vec![spe, txe, freq]),
            access: None,
            size: Some(32),
            reset_mask: None,
            reset_value: None,
            address_offset: 0x00000000,
        };

        let sess = parse::ParseSess::new();
        let mut macro_loader = DummyResolver;
        let cx = make_ext_ctxt(&sess, &mut macro_loader);

        let items = reg.gen_mmap(&cx);
        for item in items {
            println!("{}", item_to_string(&item));
        }
    }

    #[test]
    fn test_field_gen_get() {
        let register = Register {
            name: "CR".to_owned(),
            description: "Control register".to_owned(),
            address_offset: 0x00,
            size: None,
            access: None,
            reset_value: None,
            reset_mask: None,
            fields: Some(vec![
                         Field {
                             name: "RX".to_owned(),
                             description: Some("Receive enabled".to_owned()),
                             bit_range: BitRange {
                                 offset: 11,
                                 width: 1,
                             },
                             access: Some(Access::ReadWrite),
                             enumerated_values: None,
                         }])
        };
        let ref field = register.fields.as_ref().unwrap().get(0).unwrap();

        let sess = parse::ParseSess::new();
        let mut macro_loader = DummyResolver;
        let cx = make_ext_ctxt(&sess, &mut macro_loader);

        let items = field.gen_get(&cx, &register);
        assert_eq!(item_to_string(&items.get(1).unwrap()),
r"impl CrGet {
    #[inline(always)]
    pub fn rx(&self) -> bool { (self.value >> 11u32) & 1u32 != 0 }
}");
    }

    #[test]
    fn test_field_gen_update() {

        let register = Register {
            name: "CR".to_owned(),
            description: "Control register".to_owned(),
            address_offset: 0x00,
            size: None,
            access: None,
            reset_value: None,
            reset_mask: None,
            fields: Some(vec![
                         Field {
                             name: "RX".to_owned(),
                             description: Some("Receive enabled".to_owned()),
                             bit_range: BitRange {
                                 offset: 11,
                                 width: 1,
                             },
                             access: Some(Access::ReadWrite),
                             enumerated_values: None,
                         }])
        };
        let ref field = register.fields.as_ref().unwrap().get(0).unwrap();

        let sess = parse::ParseSess::new();
        let mut macro_loader = DummyResolver;
        let cx = make_ext_ctxt(&sess, &mut macro_loader);

        let items = field.gen_update(&cx, &register);
        assert_eq!(item_to_string(&items.get(1).unwrap()),
r"#[allow(dead_code, missing_docs)]
impl <'a> CrUpdate<'a> {
    #[inline(always)]
    pub fn set_rx<'b>(&'b mut self, new_value: bool) -> &'b mut CrUpdate<'a> {
        self.value =
            (self.value & !(1u32 << 11u32)) |
                ((new_value as u32) & 1u32) << 11u32;
        self.mask |= 1u32 << 11u32;
        self
    }
}");
    }

    #[test]
    fn test_field_gen_type1() {
        let field = Field {
            name: "RX".to_owned(),
            description: Some("Receive enabled".to_owned()),
            bit_range: BitRange {
                offset: 11,
                width: 1,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: None,
        };

        let builder = AstBuilder::new();
        let ty = field.gen_type();
        assert_eq!(ty, builder.ty().bool());
    }

    #[test]
    fn test_field_gen_type2() {
        let field = Field {
            name: "RX".to_owned(),
            description: Some("Receive enabled".to_owned()),
            bit_range: BitRange {
                offset: 11,
                width: 2,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: None,
        };

        let builder = AstBuilder::new();
        let ty = field.gen_type();
        assert_eq!(ty, builder.ty().u8());
    }

    #[test]
    fn test_field_gen_type3() {
        let field = Field {
            name: "PARITY".to_owned(),
            description: Some("UART Parity".to_owned()),
            bit_range: BitRange {
                offset: 2,
                width: 3,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: Some(
                EnumeratedValues {
                    name: Some("PARITY".to_owned()),
                    usage: None,
                    derived_from: None,
                    values: vec![
                        EnumeratedValue {
                            name: "NONE".to_owned(),
                            description: None,
                            value: Some(0),
                            is_default: None,
                        },
                        EnumeratedValue {
                            name: "EVEN".to_owned(),
                            description: None,
                            value: Some(2),
                            is_default: None,
                        },
                        EnumeratedValue {
                            name: "ODD".to_owned(),
                            description: None,
                            value: Some(3),
                            is_default: None,
                        },
                    ]}),
        };

        let builder = AstBuilder::new();
        let ty = field.gen_type();
        assert_eq!(ty, builder.ty().id("Parity"));
    }

    #[test]
    fn test_field_gen_type4() {
        let field = Field {
            name: "RX".to_owned(),
            description: Some("Receive enabled".to_owned()),
            bit_range: BitRange {
                offset: 11,
                width: 9,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: None,
        };

        let builder = AstBuilder::new();
        let ty = field.gen_type();
        assert_eq!(ty, builder.ty().u16());
    }

    #[test]
    fn test_field_gen_type_def1() {
        let field = Field {
            name: "PARITY".to_owned(),
            description: Some("UART Parity".to_owned()),
            bit_range: BitRange {
                offset: 2,
                width: 3,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: Some(
                EnumeratedValues {
                    name: Some("PARITY".to_owned()),
                    usage: None,
                    derived_from: None,
                    values: vec![
                        EnumeratedValue {
                            name: "NONE".to_owned(),
                            description: None,
                            value: Some(0),
                            is_default: None,
                        },
                        EnumeratedValue {
                            name: "EVEN".to_owned(),
                            description: None,
                            value: Some(2),
                            is_default: None,
                        },
                        EnumeratedValue {
                            name: "ODD".to_owned(),
                            description: None,
                            value: Some(3),
                            is_default: None,
                        },
                    ]}),
        };

        let sess = parse::ParseSess::new();
        let mut macro_loader = DummyResolver;
        let cx = make_ext_ctxt(&sess, &mut macro_loader);
        let item = field.gen_type_def(&cx);
        assert_eq!(item_to_string(&item.unwrap()),
r"#[allow(dead_code, missing_docs)]
enum Parity { None = 0u32, Even = 2u32, Odd = 3u32, }");
    }

    #[test]
    fn test_field_gen_type_def2() {
        let field = Field {
            name: "UART_PARITY".to_owned(),
            description: Some("UART Parity".to_owned()),
            bit_range: BitRange {
                offset: 2,
                width: 3,
            },
            access: Some(Access::ReadWrite),
            enumerated_values: Some(
                EnumeratedValues {
                    name: None,
                    usage: None,
                    derived_from: None,
                    values: vec![
                        EnumeratedValue {
                            name: "NONE".to_owned(),
                            description: None,
                            value: Some(0),
                            is_default: None,
                        },
                        EnumeratedValue {
                            name: "EVEN".to_owned(),
                            description: None,
                            value: Some(2),
                            is_default: None,
                        },
                        EnumeratedValue {
                            name: "ODD".to_owned(),
                            description: None,
                            value: Some(3),
                            is_default: None,
                        },
                    ]}),
        };

        let sess = parse::ParseSess::new();
        let mut macro_loader = DummyResolver;
        let cx = make_ext_ctxt(&sess, &mut macro_loader);
        let item = field.gen_type_def(&cx);
        assert_eq!(item_to_string(&item.unwrap()),
r"#[allow(dead_code, missing_docs)]
enum UartParity { None = 0u32, Even = 2u32, Odd = 3u32, }");

    }
}
