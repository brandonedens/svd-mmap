# svd-mmap

> An ARM SVD description to hardware interface in Rust.

## About

svd-mmap is a crate that defines a Rust macro which given an ARM SVD file,
which is a description of a SoC design), generates Rust software for
interfacing with the memory mapped registers for that hardware.

The best way of understanding this is through an simplified demonstration of
the conversion of SVD to Rust.  Let us suppose that we have a SVD file which
contains the following:

```xml
<?xml version="1.0" encoding="utf-8" standalone="no"?>
<device schemaVersion="1.1"
xmlns:xs="http://www.w3.org/2001/XMLSchema-instance"
xs:noNamespaceSchemaLocation="CMSIS-SVD_Schema_1_1.xsd">
  <name>STM32L4x6</name>
  <peripherals>
    <peripheral>
      <name>SPI1</name>
      <baseAddress>0x40013000</baseAddress>
      <registers>
        <register>
          <name>CR1</name>
          <description>control register 1</description>
          <addressOffset>0x0</addressOffset>
          <fields>
            <field>
              <name>LSBFIRST</name>
              <description>Frame format</description>
              <bitOffset>7</bitOffset>
              <bitWidth>1</bitWidth>
            </field>
            <field>
              <name>SPE</name>
              <description>SPI enable</description>
              <bitOffset>6</bitOffset>
              <bitWidth>1</bitWidth>
            </field>
            <field>
              <name>CPOL</name>
              <description>Clock polarity</description>
              <bitOffset>1</bitOffset>
              <bitWidth>1</bitWidth>
            </field>
            <field>
              <name>CPHA</name>
              <description>Clock phase</description>
              <bitOffset>0</bitOffset>
              <bitWidth>1</bitWidth>
            </field>
          </fields>
		</register>
	  </registers>
    </peripheral>
  </peripherals>
</device>
```

We can expect the following Rust software to be generated.

```rust
pub mod stm32l4x6 {
    pub mod spi1 {
        use volatile_cell::VolatileCell;
        use core::ops::Drop;
        #[allow(dead_code, missing_docs)]
        #[repr(C)]
        pub struct Spi1 {
            pub cr1: Cr1,
        }
        #[allow(dead_code, missing_docs)]
        #[repr(C)]
        pub struct Cr1 {
            value: VolatileCell<u32>,
        }
        #[allow(dead_code, missing_docs)]
        #[derive(Clone)]
        pub struct Cr1Get {
            value: u32,
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1 {
            #[inline(always)]
            pub fn get(&self) -> Cr1Get { Cr1Get::new(self) }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1Get {
            #[inline(always)]
            pub fn new(reg: &Cr1) -> Cr1Get {
                Cr1Get{value: reg.value.get(),}
            }
        }
        #[allow(dead_code, missing_docs)]
        pub struct Cr1Update<'a> {
            value: u32,
            mask: u32,
            write_only: bool,
            reg: &'a Cr1,
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1 {
            #[inline(always)]
            pub fn ignoring_state(&self) -> Cr1Update {
                Cr1Update::new_ignoring_state(self)
            }
        }
        #[allow(dead_code, missing_docs)]
        impl <'a> Drop for Cr1Update<'a> {
            #[inline(always)]
            fn drop(&mut self) {
                let clear_mask: u32 = 1u32 as u32;
                if self.mask != 0 {
                    let v: u32 =
                        if self.write_only { 0 } else { self.reg.value.get() }
                            & !clear_mask & !self.mask;
                    self.reg.value.set(self.value | v);
                }
            }
        }
        #[allow(dead_code, missing_docs)]
        impl <'a> Cr1Update<'a> {
            #[inline(always)]
            pub fn new(reg: &'a Cr1) -> Cr1Update<'a> {
                Cr1Update{value: 0, mask: 0, write_only: false, reg: reg,}
            }
            #[inline(always)]
            pub fn new_ignoring_state(reg: &'a Cr1) -> Cr1Update<'a> {
                Cr1Update{value: 0, mask: 0, write_only: true, reg: reg,}
            }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1 {
            #[inline(always)]
            pub fn lsbfirst(&self) -> bool { Cr1Get::new(self).lsbfirst() }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1Get {
            #[inline(always)]
            pub fn lsbfirst(&self) -> bool {
                (self.value >> 7u32) & 1u32 != 0
            }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1 {
            #[inline(always)]
            pub fn spe(&self) -> bool { Cr1Get::new(self).spe() }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1Get {
            #[inline(always)]
            pub fn spe(&self) -> bool { (self.value >> 6u32) & 1u32 != 0 }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1 {
            #[inline(always)]
            pub fn cpol(&self) -> bool { Cr1Get::new(self).cpol() }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1Get {
            #[inline(always)]
            pub fn cpol(&self) -> bool { (self.value >> 1u32) & 1u32 != 0 }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1 {
            #[inline(always)]
            pub fn cpha(&self) -> bool { Cr1Get::new(self).cpha() }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1Get {
            #[inline(always)]
            pub fn cpha(&self) -> bool { (self.value >> 0u32) & 1u32 != 0 }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1 {
            #[inline(always)]
            pub fn set_lsbfirst<'a>(&'a self, new_value: bool)
             -> Cr1Update<'a> {
                let mut setter: Cr1Update = Cr1Update::new(self);
                setter.set_lsbfirst(new_value);
                setter
            }
        }
        #[allow(dead_code, missing_docs)]
        impl <'a> Cr1Update<'a> {
            #[inline(always)]
            pub fn set_lsbfirst<'b>(&'b mut self, new_value: bool)
             -> &'b mut Cr1Update<'a> {
                self.value =
                    (self.value & !(1u32 << 7u32)) |
                        ((new_value as u32) & 1u32) << 7u32;
                self.mask |= 1u32 << 7u32;
                self
            }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1 {
            #[inline(always)]
            pub fn set_spe<'a>(&'a self, new_value: bool) -> Cr1Update<'a> {
                let mut setter: Cr1Update = Cr1Update::new(self);
                setter.set_spe(new_value);
                setter
            }
        }
        #[allow(dead_code, missing_docs)]
        impl <'a> Cr1Update<'a> {
            #[inline(always)]
            pub fn set_spe<'b>(&'b mut self, new_value: bool)
             -> &'b mut Cr1Update<'a> {
                self.value =
                    (self.value & !(1u32 << 6u32)) |
                        ((new_value as u32) & 1u32) << 6u32;
                self.mask |= 1u32 << 6u32;
                self
            }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1 {
            #[inline(always)]
            pub fn set_cpol<'a>(&'a self, new_value: bool) -> Cr1Update<'a> {
                let mut setter: Cr1Update = Cr1Update::new(self);
                setter.set_cpol(new_value);
                setter
            }
        }
        #[allow(dead_code, missing_docs)]
        impl <'a> Cr1Update<'a> {
            #[inline(always)]
            pub fn set_cpol<'b>(&'b mut self, new_value: bool)
             -> &'b mut Cr1Update<'a> {
                self.value =
                    (self.value & !(1u32 << 1u32)) |
                        ((new_value as u32) & 1u32) << 1u32;
                self.mask |= 1u32 << 1u32;
                self
            }
        }
        #[allow(dead_code, missing_docs)]
        impl Cr1 {
            #[inline(always)]
            pub fn set_cpha<'a>(&'a self, new_value: bool) -> Cr1Update<'a> {
                let mut setter: Cr1Update = Cr1Update::new(self);
                setter.set_cpha(new_value);
                setter
            }
        }
        #[allow(dead_code, missing_docs)]
        impl <'a> Cr1Update<'a> {
            #[inline(always)]
            pub fn set_cpha<'b>(&'b mut self, new_value: bool)
             -> &'b mut Cr1Update<'a> {
                self.value =
                    (self.value & !(1u32 << 0u32)) |
                        ((new_value as u32) & 1u32) << 0u32;
                self.mask |= 1u32 << 0u32;
                self
            }
        }
        #[allow(dead_code)]
        extern "C" {
            #[link_name = "mmap_stm32l4x6_spi1"]
            pub static SPI1: Spi1;
        }
    }
}
```

## Usage

Here is basic usage of the SPI1 peripheral from STM32L4x6 SVD definition.

```rust
svd_mmap!("STM32L4x6.svd");

use stm32l4x6::spi::SPI1;

fn main() {
	unsafe {
		SPI1.cr1.set_lsbfirst(true).set_cpol(true).set_cpha(true);
		SPI1.cr1.set_spe(true);
	}
}
```

which generates the following armv7m thumb2 binary:

```asm
08000000 <main>:
 8000000:       f243 0000       movw    r0, #12288      ; 0x3000
 8000004:       f2c4 0001       movt    r0, #16385      ; 0x4001
 8000008:       6801            ldr     r1, [r0, #0]
 800000a:       f041 0183       orr.w   r1, r1, #131    ; 0x83
 800000e:       6001            str     r1, [r0, #0]
 8000010:       6801            ldr     r1, [r0, #0]
 8000012:       f021 0141       bic.w   r1, r1, #65     ; 0x41
 8000016:       f041 0140       orr.w   r1, r1, #64     ; 0x40
 800001a:       6001            str     r1, [r0, #0]
 800001c:       2000            movs    r0, #0
 800001e:       4770            bx      lr
```

Given any SVD file you can immediately generate a Rust hardware definition
using this software by executing:

cargo run ~/path/to/svd/file.svd

which will print to standard out the Rust software that would be generated for
that definition. This information will make it easier to determine what the
macro will generate as a Rust interface is for the given hardware.

## Thanks

Many thanks got to the Zinc.rs project specifically the ioreg macro's
definition of Rust software to emit as this software is functionally
equivalent.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the
work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
