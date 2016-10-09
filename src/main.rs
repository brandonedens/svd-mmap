// Copyright 2016 by Brandon Edens. All Rights Reserved
// Author: Brandon Edens <brandonedens@gmail.com>
// Date: 2016-09-23

//! Command line software for generating Rust software to interface with memory map defined in SVD
//! file.

#![feature(rustc_private)]

extern crate aster;
extern crate clap;
extern crate svd_parser as svd;
#[allow(plugin_as_library)]
extern crate svd_mmap;
extern crate syntax;

use aster::name::ToName;
use clap::App;
use std::fs::File;
use std::io::prelude::*;
use svd::Device;
use svd_mmap::gen_device;
use syntax::codemap;
use syntax::ext::base::{DummyResolver, ExtCtxt};
use syntax::ext::expand;
use syntax::parse;
use syntax::print::pprust::item_to_string;

fn main() {

    let matches = App::new("svd-mmap")
        .version("0.1")
        .author("Brandon Edens <brandonedens@gmail.com>")
        .about("Generate memory map from SVD")
        .args_from_usage(
            "<INPUT_SVD>    'The SVD file to use as input'"
            )
        .get_matches();

    // Read out the SVD file.
    let svd_filename = matches.value_of("INPUT_SVD").unwrap();
    let mut svd_file = File::open(svd_filename).unwrap();
    let mut s = String::new();
    svd_file.read_to_string(&mut s).unwrap();

    // Generate SVD device data from SVD XML.
    let dev = Device::parse(&s);

    // Generate Rust software for interfacing to memory mapped hardware.
    let sess = parse::ParseSess::new();
    let mut macro_loader = DummyResolver;
    let mut cx = make_ext_ctxt(&sess, &mut macro_loader);
    let items = gen_device(&mut cx, &dev);

    // Print generated Rust to standard output.
    for item in items {
        println!("{}", item_to_string(&item));
    }
}

/// Context used for generating Rust software.
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

