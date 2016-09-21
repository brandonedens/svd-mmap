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

// TODO remove
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

// TODO combine these two functions.
fn read_only(field: &Field, reg: &Register) -> bool {
    if let Some(access) = field.access.as_ref() {
        return *access == Access::ReadOnly;
    }

    if let Some(access) = reg.access.as_ref() {
        return *access == Access::ReadOnly;
    }

    return false;
}

fn write_only(field: &Field, reg: &Register) -> bool {
    if let Some(access) = field.access.as_ref() {
        return *access == Access::WriteOnly;
    }

    if let Some(access) = reg.access.as_ref() {
        return *access == Access::WriteOnly;
    }

    return false;
}

fn field_size_to_ty(field: &Field) -> syntax::ptr::P<syntax::ast::Ty> {
    let builder = aster::AstBuilder::new();

    let field_ty = match field.bit_range.width {
        1 => builder.ty().bool(),
        2...8 => builder.ty().u8(),
        9...16 => builder.ty().u16(),
        17...32 => builder.ty().u32(),
        33...64 => builder.ty().u64(),
        _ => panic!("Unknown bit width"),
    };
    field_ty
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
            let periph_name = builder.id(periph.name.to_snake_case());

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
                    let periph_name = builder.id(periph_name.to_snake_case());
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
            v.append(&mut gen_reg_field_impl(cx, reg));
        }
    }

    v
}

/// Generate implementation items for each field in register.
fn gen_reg_field_impl(cx: &ExtCtxt, reg: &Register) -> Vec<P<syntax::ast::Item>> {
    let mut v = Vec::new();
    let builder = aster::AstBuilder::new();

    let reg_type_name = builder.id(reg.name.to_pascal_case());
    let reg_name_get = builder.id(reg.name.to_pascal_case() + "Get");
    let reg_name_update = builder.id(reg.name.to_pascal_case() + "Update");

    let item = quote_item!(&cx,
                           #[allow(dead_code, missing_docs)]
                           #[repr(C)]
                           pub struct $reg_type_name {
                                value: VolatileCell<u32>,
                           }).unwrap();
    v.push(item);

    let item = quote_item!(&cx,
                           #[allow(dead_code, missing_docs)]
                           impl $reg_type_name {

                               #[inline(always)]
                               pub fn get(&self) -> $reg_name_get {
                                   $reg_name_get::new(self)
                               }

                               #[inline(always)]
                               pub fn ignoring_state(&self) -> $reg_name_update {
                                   $reg_name_update::new_ignoring_state(self)
                               }
                           }).unwrap();
    v.push(item);

    let item = quote_item!(&cx,
                           #[allow(dead_code, missing_docs)]
                           #[derive(Clone)]
                           pub struct $reg_name_get {
                               value: u32,
                           }).unwrap();
    v.push(item);

    let item = quote_item!(&cx,
                           #[allow(dead_code, missing_docs)]
                           impl $reg_name_get {
                               #[inline(always)]
                               pub fn new(reg: &$reg_type_name) -> $reg_name_get {
                                   $reg_name_get { value: reg.value.get() }
                               }
                           }).unwrap();
    v.push(item);

    let item = quote_item!(&cx,
                           #[allow(dead_code, missing_docs)]
                           pub struct $reg_name_update<'a> {
                               value: u32,
                               mask: u32,
                               write_only: bool,
                               reg: &'a $reg_type_name,
                           }).unwrap();
    v.push(item);

    let item = quote_item!(&cx,
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
        }).unwrap();
    v.push(item);

    let item = quote_item!(&cx,
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
        }).unwrap();
    v.push(item);

    if let Some(fields) = reg.fields.as_ref() {
        for field in fields {
            let field_ty = field_size_to_ty(field);
            let bit_offset = field.bit_range.offset;
            let bit_width = field.bit_range.width;

            if !read_only(field, reg) {
                let field_name = builder.id("set_".to_string() + &field.name.to_snake_case());

                let item = quote_item!(&cx,
                                       #[allow(dead_code, missing_docs)]
                                       impl $reg_type_name {
                                           #[inline(always)]
                                           pub fn $field_name<'a>(&'a self, new_value: $field_ty) -> $reg_name_update<'a> {
                                               let mut setter: $reg_name_update = $reg_name_update::new(self);
                                               setter.$field_name(new_value);
                                               setter
                                           }
                                       }
                                      ).unwrap();
                v.push(item);

                let item = quote_item!(&cx,
                    #[allow(dead_code, missing_docs)]
                    impl<'a> $reg_name_update<'a> {
                        #[inline(always)]
                        pub fn $field_name<'b>(&'b mut self, new_value: $field_ty) -> &'b mut $reg_name_update<'a> {
                            self.value = (self.value & !($bit_width << $bit_offset)) |
                                ((new_value as u32) & $bit_width) << $bit_offset;
                            self.mask |= $bit_width << $bit_offset;
                            self
                        }
                    }).unwrap();
                v.push(item);
            }

            if !write_only(field, reg) {
                let field_name = builder.id(field.name.to_snake_case());
                let item = quote_item!(&cx,
                                       #[allow(dead_code, missing_docs)]
                                       impl $reg_type_name {
                                           #[inline(always)]
                                           pub fn $field_name(&self) -> $field_ty {
                                               $reg_name_get::new(self).$field_name()
                                           }
                                       }
                                      ).unwrap();
                v.push(item);

                if field.bit_range.width == 1 {
                    let item = quote_item!(&cx,
                                           #[allow(dead_code, missing_docs)]
                                           impl $reg_name_get {
                                               #[inline(always)]
                                               pub fn $field_name(&self) -> $field_ty {
                                                   (self.value >> $bit_offset) & $bit_width != 0
                                               }
                                           }).unwrap();
                    v.push(item);
                } else {
                    let item = quote_item!(&cx,
                                           #[allow(dead_code, missing_docs)]
                                           impl $reg_name_get {
                                               #[inline(always)]
                                               pub fn $field_name(&self) -> $field_ty {
                                                   ((self.value >> $bit_offset) & $bit_width) as $field_ty
                                               }
                                           }).unwrap();
                    v.push(item);
                }
            }
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

    use aster::name::ToName;
    use std::fs::File;
    use std::io::prelude::*;
    use svd::{Access, BitRange, Device, Field, Peripheral, Register};
    use syntax::codemap;
    use syntax::ext::base::{DummyResolver, ExtCtxt};
    use syntax::ext::expand;
    use syntax::parse;
    use syntax::print::pprust::item_to_string;

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

        let items = super::gen_reg_field_impl(&cx, &reg);
        for item in items {
            println!("{}", item_to_string(&item));
        }
    }
}
