use super::super::{Node, Options, Root};
use super::eval::static_size;
use super::*;

const MDF4: &str = include_str!("../../../../templates/mdf4.hbt");

fn opts() -> Options {
    Options { endian: Endian::Little, encoding: encoding_rs::WINDOWS_1252 }
}

fn run(src: &str, root: Root, base: u64, data: &[u8]) -> Node {
    let t = parse(src).unwrap_or_else(|e| panic!("{e}"));
    apply(&t, data, root, base, &opts()).unwrap()
}

fn ty(name: &str) -> Root {
    Root::Type(name.into())
}

fn child<'n>(n: &'n Node, name: &str) -> &'n Node {
    n.children
        .iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("no child {name} in {:?}", n.children.iter().map(|c| &c.name).collect::<Vec<_>>()))
}

fn path<'n>(n: &'n Node, p: &str) -> &'n Node {
    p.split('/').fold(n, child)
}

fn err(src: &str) -> TemplateError {
    match parse(src) {
        Ok(_) => panic!("expected a parse error for:\n{src}"),
        Err(e) => e,
    }
}

/// An MDF4 block: header, links, data.
fn block(id: &str, links: &[u64], data: &[u8]) -> Vec<u8> {
    let mut b = id.as_bytes().to_vec();
    b.extend(0u32.to_le_bytes());
    b.extend((24 + 8 * links.len() as u64 + data.len() as u64).to_le_bytes());
    b.extend((links.len() as u64).to_le_bytes());
    for l in links {
        b.extend(l.to_le_bytes());
    }
    b.extend(data);
    b
}

fn pad8(v: &mut Vec<u8>) {
    while !v.len().is_multiple_of(8) {
        v.push(0);
    }
}

/// ID at 0, HD at 64, TX at 168, DG at 200, CC (LIN) at 264, CG at 360.
fn mdf_file() -> Vec<u8> {
    let mut f = Vec::new();
    f.extend(b"MDF     4.10    test    ");
    f.extend([0; 4]);
    f.extend(410u16.to_le_bytes());
    f.extend([0; 30]);
    f.extend([0; 4]);
    assert_eq!(f.len(), 64);
    let mut hd = vec![0u8; 32];
    hd[8..10].copy_from_slice(&60i16.to_le_bytes());
    f.extend(block("##HD", &[200, 0, 0, 0, 0, 168], &hd));
    assert_eq!(f.len(), 168);
    f.extend(block("##TX", &[], b"hello\0"));
    pad8(&mut f);
    assert_eq!(f.len(), 200);
    f.extend(block("##DG", &[0, 360, 0, 0], &[0; 8]));
    assert_eq!(f.len(), 264);
    let mut cc = vec![1, 0, 0, 0, 0, 0, 2, 0];
    cc.extend(0f64.to_le_bytes());
    cc.extend(100f64.to_le_bytes());
    cc.extend(0.5f64.to_le_bytes());
    cc.extend(2f64.to_le_bytes());
    f.extend(block("##CC", &[168, 0, 0, 0], &cc));
    assert_eq!(f.len(), 360);
    // CG with the vlsd and remote_master flags and one link too many
    let mut cg = vec![0u8; 16];
    cg.extend(0b1001u16.to_le_bytes());
    cg.extend([0; 6]);
    cg.extend(77u64.to_le_bytes());
    f.extend(block("##CG", &[0, 0, 0, 0, 0, 0, 360, 4242], &cg));
    f
}

#[test]
fn detection() {
    assert!(detect("#lang blocks 1\n").unwrap());
    assert!(detect("\u{FEFF}  #lang\tblocks  1  \r\nx").unwrap());
    assert!(!detect("struct A { int x; };").unwrap());
    assert!(!detect("").unwrap());
    let e = detect("#lang blocks 2\n").unwrap_err();
    assert_eq!(e.msg, "unsupported block template version 2 (supported: 1)");
    assert!(detect("#lang c 1").is_err());
}

#[test]
fn bundled_mdf4_parses() {
    let t = parse(MDF4).unwrap_or_else(|e| panic!("{e}"));
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    assert!(t.file.is_some());
    assert_eq!(t.root_names[0], "Id");
    for r in &t.root_names {
        assert!(t.targets(r).is_some(), "{r}");
    }
}

#[test]
fn file_and_links() {
    let d = mdf_file();
    let n = run(MDF4, Root::Instances, 0, &d);
    assert_eq!(n.name, "template");
    assert_eq!(path(&n, "id/file_id").value, "\"MDF     \"");
    assert!(path(&n, "id/file_id").error.is_none());
    let hd = child(&n, "hd");
    assert_eq!((hd.offset, hd.size, hd.ty.as_str()), (64, 104, "HD"));
    // sections in frame order
    let names: Vec<&str> = hd.children.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["header", "links", "data"]);
    assert!(path(hd, "data/tz_offset_min").value.starts_with("60 "));
    let dg = path(hd, "links/first_dg");
    assert_eq!(dg.value, "→ 0xC8 (DG)");
    assert_eq!(dg.ty, "-> DG");
    assert_eq!(dg.link.as_ref().unwrap().offset, 200);
    assert_eq!(path(hd, "links/file_history").value, "null");
    assert!(path(hd, "links/file_history").link.is_none());
    let c = path(hd, "links/comment");
    assert_eq!(c.value, "→ 0xA8 (TX | MD)");
    assert_eq!(c.link.as_ref().unwrap().ty, "TX | MD");
    // the link slot is selected, not the target
    assert_eq!((c.offset, c.size), (64 + 24 + 40, 8));
}

#[test]
fn dispatch() {
    let d = mdf_file();
    let tx = run(MDF4, ty("TX | MD"), 168, &d);
    assert_eq!(tx.dispatched.as_deref(), Some("TX"));
    assert!(tx.error.is_none());
    assert_eq!(path(&tx, "data/text").value, "\"hello\"");
    // the extent is 30 bytes; the next block starts on an 8-byte boundary
    assert_eq!(tx.size, 30);
    assert_eq!(tx.next.as_ref().unwrap().offset, 200);
    assert_eq!(tx.next.as_ref().unwrap().ty, BY_MAGIC);

    // another block of the same frame
    let dg = run(MDF4, ty("TX | MD"), 200, &d);
    assert_eq!(dg.dispatched.as_deref(), Some("DG"));
    assert_eq!(dg.error.as_deref(), Some("expected TX | MD, found DG"));

    // nothing matches: the first candidate with its failed check
    let bad = run(MDF4, ty("CN"), 3, &d);
    assert_eq!(bad.error.as_deref(), Some("unknown block"));
    assert_eq!(path(&bad, "header/id").error.as_deref(), Some("check failed: id == magic"));

    let m = run(MDF4, Root::ByMagic, 3, &d);
    assert_eq!(m.error.as_deref(), Some("unknown block"));
    assert!(m.children.is_empty());
    let m = run(MDF4, Root::ByMagic, 264, &d);
    assert_eq!(m.ty, "CC");
    assert!(m.dispatched.is_none());

    // a struct is decoded without dispatch
    let id = run(MDF4, ty("Id"), 0, &d);
    assert_eq!(id.size, 64);
    assert!(id.next.is_none());
}

#[test]
fn match_and_conditional_links() {
    let d = mdf_file();
    let cc = run(MDF4, ty("CC"), 264, &d);
    assert_eq!(path(&cc, "data/type").value, "LIN (1)");
    assert_eq!(path(&cc, "data/a").value, "2");
    assert!(child(&cc, "data").children.iter().all(|c| c.name != "val"));
    assert_eq!(path(&cc, "links/name").value, "→ 0xA8 (TX)");
    // ref_count is 0: an empty array needs no slots
    assert_eq!(path(&cc, "links/refs").value, "[0]");
    assert!(path(&cc, "links/refs").error.is_none());

    let cg = run(MDF4, ty("CG"), 360, &d);
    assert_eq!(path(&cg, "data/flags").value, "0x9 {vlsd, remote_master}");
    assert!(path(&cg, "data/vlsd_size").value.starts_with("77 "));
    assert!(child(&cg, "data").children.iter().all(|c| c.name != "data_bytes"));
    // conditional link, then the extra slot shown raw
    assert_eq!(path(&cg, "links/master").value, "→ 0x168 (CG)");
    assert!(path(&cg, "links/links[7]").value.starts_with("4242 "));
}

#[test]
fn slot_mismatch() {
    let src = "#lang blocks 1\n\
        frame F { h { id: u8 where id == 1, n: u8 }\n links: [u32; n] }\n\
        block B: F { links { a: -> B, b: [-> B; 3], c: -> B } }";
    let data = [1, 2, 4, 0, 0, 0, 0, 0, 0, 0];
    let n = run(src, ty("B"), 0, &data);
    assert_eq!(path(&n, "links/a").link.as_ref().unwrap().offset, 4);
    let b = path(&n, "links/b");
    assert_eq!(b.children.len(), 1);
    assert_eq!(b.error.as_deref(), Some("only 1 of 3 slots present"));
    assert_eq!(path(&n, "links/c").value, "not present");
    assert!(n.next.is_some());
}

#[test]
fn link_targets_with_at() {
    let src = "#lang blocks 1\n\
        struct E { v: u8 }\n\
        frame F { links: [u8; 2] }\n\
        block B: F { links { first: -> E @ 0x10 + first * 2, far: -> E } }";
    let n = run(src, ty("B"), 0, &[0, 200, 0, 0]);
    // with '@' a raw 0 is a valid link
    let first = path(&n, "links/first");
    assert_eq!(first.value, "→ 0x10 (E) (raw 0)");
    assert!(first.error.as_deref().unwrap().contains("outside the file"));
    assert_eq!(path(&n, "links/far").error.as_deref(), Some("link target 0xC8 is outside the file"));
}

#[test]
fn sections_and_bounds() {
    let src = "#lang blocks 1\n\
        frame F { h { len: u8 }\n body: size(len) }\n\
        block B: F { body { a: u8, s: ascii, rest: [u16; ..] } }\n\
        block R: F { }";
    // len 7: a, "hi\0", one u16, then a u16 that crosses the end of the section
    let data = [7, 9, b'h', b'i', 0, 1, 0, 7, 0xEE];
    let n = run(src, ty("B"), 0, &data);
    assert_eq!(path(&n, "body/s").value, "\"hi\"");
    assert_eq!(path(&n, "body/s").size, 3);
    let rest = path(&n, "body/rest");
    assert_eq!(rest.children.len(), 2);
    assert_eq!(rest.children[1].error.as_deref(), Some("beyond end of section"));
    assert_eq!(rest.error.as_deref(), Some("extends beyond end of section"));
    assert_eq!(n.size, 8);

    let n = run(src, ty("B"), 0, &[2, 9, 0xC4, 0]);
    assert_eq!(path(&n, "body/s").value, "\"\\xC4\"");
    assert_eq!(path(&n, "body/s").error, None);
    // unused bytes
    let n = run(src, ty("B"), 0, &[1, 9]);
    assert!(path(&n, "body").children.iter().any(|c| c.name == "s"));
    let n = run("#lang blocks 1\nframe F { h { len: u8 }\n body: size(len) }\nblock B: F { body { a: u8 } }", ty("B"), 0, &[4, 1, 2, 3, 4]);
    assert_eq!(path(&n, "body/⋯ 3 unparsed bytes").size, 3);
    // a block without a body shows the raw bytes
    let n = run(src, ty("R"), 0, &[2, 0xAB, 0xCD]);
    assert_eq!(child(&n, "body").value, "AB CD");
}

#[test]
fn values_and_expressions() {
    let src = r#"#lang blocks 1
endian big
const N = 2
enum E: u8 { A = 1, B, C = 0x10 }
flags F: u8 { x = 0, y = 7 }
struct S {
    e: E
    f: F
    n: u16le
    tag: utf16(N)
    t2: ascii(4) where t2 == "AB"
    bad: u8 where bad == 1
    arr: [u8; n / (e - 2)]
    if f.y { hi: u8 } else if f.x { lo: u8 }
    v: u8 where (hi > 0) == 1
    w: u8 where lo == 0
}"#;
    let data = [2, 0x81, 3, 0, 0, b'O', 0, b'K', b'A', b'B', b' ', 0, 5, 9, 10, 11];
    let n = run(src, ty("S"), 0, &data);
    assert_eq!(child(&n, "e").value, "B (2)");
    assert_eq!(child(&n, "f").value, "0x81 {x, y}");
    assert!(child(&n, "n").value.starts_with("3 "));
    assert_eq!(child(&n, "tag").value, "\"OK\"");
    assert!(child(&n, "t2").error.is_none());
    assert_eq!(child(&n, "bad").error.as_deref(), Some("check failed: bad == 1"));
    assert_eq!(child(&n, "arr").error.as_deref(), Some("division by zero"));
    assert_eq!(child(&n, "hi").value, "9 (0x09)");
    assert!(child(&n, "v").error.is_none());
    // 'lo' is in the branch that was not decoded
    assert_eq!(child(&n, "w").error.as_deref(), Some("check failed: lo == 0"));
    assert_eq!(n.size, 16);
}

#[test]
fn sizeof_and_consts() {
    let src = "#lang blocks 1\n\
        const A = B * 2\nconst B = sizeof(P)\nconst S = \"x\"\n\
        struct P { a: u32, b: [u16; 3] }\n\
        struct Q { pad: [u8; A] }";
    let t = parse(src).unwrap();
    assert_eq!(t.consts["B"], Value::Int(10));
    assert_eq!(t.consts["A"], Value::Int(20));
    assert_eq!(t.consts["S"], Value::Str("x".into()));
    assert_eq!(static_size(&t, &Type::Struct(1)), Some(20));
}

#[test]
fn qualified_and_ambiguous_enum_items() {
    let src = "#lang blocks 1\n\
        enum A: u8 { X = 1, Y }\nenum B: u8 { X = 5 }\n\
        struct S { a: A, match a { X => { one: u8 }, _ => { } }, b: [u8; B.X] }";
    let n = run(src, ty("S"), 0, &[1, 7, 0, 0, 0, 0, 0]);
    assert!(child(&n, "one").value.starts_with("7 "));
    assert_eq!(child(&n, "b").size, 5);
    let e = err("#lang blocks 1\nenum A: u8 { X }\nenum B: u8 { X }\nstruct S { n: [u8; X] }");
    assert!(e.msg.contains("ambiguous enum item 'X'"), "{e}");
}

#[test]
fn parse_errors() {
    let cases: &[(&str, &str)] = &[
        ("struct A { x: u8 }\nstruct A { y: u8 }", "duplicate declaration 'A'"),
        ("struct A { x: Foo }", "unknown type 'Foo'"),
        ("struct A { x: -> A }", "link types are only allowed in slot sections"),
        ("struct A { x: u8 @ 4 }", "'@' is only allowed on fields of 'file { }'"),
        ("struct A { x: u8 }\nendian big", "'endian' must come once"),
        ("struct A { x: u8, x: u16 }", "duplicate field 'x'"),
        ("struct A { x: u8, if x { x: u8 } }", "duplicate field 'x'"),
        ("struct A { b: A }", "'A' contains itself"),
        ("struct A { b: B }\nstruct B { a: [A; 2] }", "contains itself"),
        ("struct A { x: u8 }\nframe F { l: [u8; 1] }\nblock B: F { l { a: -> A | B } }", "single target"),
        ("struct A { n: u8, d: [u8; n] }\nconst C = sizeof(A)", "depends on file data"),
        ("struct E { }\nstruct A { d: [E; ..] }", "size is not 0"),
        ("frame F(m) { h { x: u8 } }\nblock B: F { }", "takes 1 argument(s), 0 given"),
        ("frame F { h { x: u8 } }\nblock B: F { h { y: u8 } }", "cannot be changed"),
        ("frame F { h { x: u8 } }\nblock B: F { z { y: u8 } }", "has no section 'z'"),
        ("flags F: u8 { a = 8 }", "out of range"),
        ("flags F: u8 { a = 1, b = 1 }", "used by both"),
        ("frame F align 3 { h { x: u8 } }", "power of two"),
        ("const A = B\nconst B = A", "defined in terms of itself"),
        ("alias A = B\nalias B = A", "refers to itself"),
        ("struct S { x: u8 }\nalias A = S", "must be blocks"),
        ("struct A { x: u8, match x { FILE_SIZE => { } } }", "match patterns must be constant"),
        ("file { x: u8 }\nfile { y: u8 }", "more than one"),
        ("frame F { l: [u8; 1] }\nblock B: F { l { a: u16 } }", "slot section entry"),
        ("frame F { l: [u8; 1] }\nblock B: F { l { a: u8 @ 1 } }", "only allowed on links"),
        ("struct A { else: u8 }", "keyword"),
        ("frame F(m) { h { x: u8 } }\nblock B: F(FILE_SIZE) { }", "non-constant argument"),
        ("struct A { x: u8 y: u8 }", "expected a line break"),
    ];
    for (src, msg) in cases {
        let e = err(&format!("#lang blocks 1\n{src}"));
        assert!(e.msg.contains(msg), "{src}\n  got: {e}\n  expected: {msg}");
    }
    // positions count the #lang line
    let e = err("#lang blocks 1\n\nstruct A {\n  x: Foo\n}");
    assert_eq!((e.line, e.col), (4, 6));
}

#[test]
fn separators_and_layout() {
    // commas, line breaks, trailing separators and comments mix freely
    let src = "#lang blocks 1\r\n// c\nstruct A {\r\n  a: u8, b: u8,\n\n  /* x */ c: u8,\n}\n\n";
    let n = run(src, ty("A"), 0, &[1, 2, 3]);
    assert_eq!(n.children.len(), 3);
    let src = "#lang blocks 1\nstruct A { n: u8\n d: [u8; (n\n - 1)] }";
    assert_eq!(child(&run(src, ty("A"), 0, &[3, 0, 0]), "d").size, 2);
}

#[test]
fn ambiguous_dispatch_warning() {
    let src = "#lang blocks 1\n\
        frame F { h { x: u8 } }\nframe G { h { x: u8 where x == 1 }\n l: [u8; 1] }\n\
        block A: F { }\nblock B: F { }\nblock C: G { l { t: -> A | B } }";
    let t = parse(src).unwrap();
    assert_eq!(t.warnings.len(), 2, "{:?}", t.warnings);
}

#[test]
fn inline_blocks_see_enclosing_fields() {
    let src = "#lang blocks 1\n\
        frame F(k) { h { v: u8 where v == k + base } }\n\
        block B: F(1) { }\n\
        file { base: u8, b: B @ 4, after: u8 }";
    let n = run(src, Root::Instances, 0, &[2, 0, 0, 0, 3, 9]);
    assert!(path(&n, "b/h/v").error.is_none());
    assert_eq!(child(&n, "after").offset, 5);
}

#[test]
fn help_example() {
    // the example in the help dialog (ui/index.html)
    let html = include_str!("../../../../ui/index.html");
    let start = html.find("<pre>#lang blocks 1").expect("help example") + "<pre>".len();
    let end = start + html[start..].find("</pre>").unwrap();
    let src = html[start..end].replace("&gt;", ">").replace("&lt;", "<").replace("&amp;", "&");
    let t = parse(&src).unwrap_or_else(|e| panic!("{e}"));
    assert!(t.warnings.is_empty());
    let mut d = vec![0u8; 64];
    d.extend(b"##HD");
    d.extend((20u64 + 16 + 5).to_le_bytes());
    d.extend(2u64.to_le_bytes());
    d.extend([0u64.to_le_bytes(), 64u64.to_le_bytes()].concat());
    d.extend([8, 1, 0, 0, 0]);
    let n = apply(&t, &d, Root::Instances, 0, &opts()).unwrap();
    assert!(path(&n, "hd/data/index").value.starts_with("1 "));
    assert_eq!(path(&n, "hd/links/comment").value, "→ 0x40 (TX | HD)");
}

const BLF: &str = include_str!("../../../../templates/blf.hbt");

/// A BLF object: base header, the v1 (32 bytes) or v2 (40 bytes) header
/// extension, the body and the padding.
fn lobj(header_size: u16, ty: u32, body: &[u8]) -> Vec<u8> {
    let size = header_size as u32 + body.len() as u32;
    let mut v = b"LOBJ".to_vec();
    v.extend(header_size.to_le_bytes());
    v.extend((if header_size == 40 { 2u16 } else { 1u16 }).to_le_bytes());
    v.extend(size.to_le_bytes());
    v.extend(ty.to_le_bytes());
    match header_size {
        32 => {
            v.extend(1u32.to_le_bytes()); // time_10us
            v.extend([0, 0, 0, 0]); // client index, object version
            v.extend(12_345u64.to_le_bytes());
        }
        40 => {
            v.extend(2u32.to_le_bytes()); // time_1ns
            v.extend([0, 0, 0, 0]);
            v.extend(12_345u64.to_le_bytes());
            v.extend(0u64.to_le_bytes());
        }
        _ => {}
    }
    v.extend(body);
    if ty != 101 {
        v.resize(v.len() + size as usize % 4, 0);
    }
    v
}

fn can_body(extra: usize) -> Vec<u8> {
    let mut b = vec![1, 0, 0x01, 8]; // channel 1, tx, dlc 8
    b.extend(0x8000_0123u32.to_le_bytes()); // extended id 0x123
    b.extend(1..=8u8);
    b.resize(b.len() + extra, 0);
    b
}

fn check_no_errors(n: &Node) {
    assert!(n.error.is_none(), "{}: {:?}", n.name, n.error);
    n.children.iter().for_each(check_no_errors);
}

#[test]
fn bundled_blf() {
    use std::io::Write;
    let t = parse(BLF).unwrap_or_else(|e| panic!("{e}"));
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    assert!(t.file.is_none());

    let mut d = b"LOGG".to_vec();
    d.extend(144u32.to_le_bytes());
    d.extend(4_070_100u32.to_le_bytes());
    d.extend([5, 6, 1, 2]);
    d.resize(144, 0);

    let can = lobj(40, 1, &can_body(0));
    let mut fd64 = vec![1, 5, 5, 0]; // channel, dlc, valid bytes, tx count
    fd64.resize(40, 0);
    fd64.extend([0xA, 0xB, 0xC, 0xD, 0xE]);
    let mut fd = can_body(0)[..8].to_vec();
    fd.extend([0; 4]); // frame length
    fd.extend([0, 0x03, 12, 0, 0, 0, 0, 0]); // arb bits, edl+brs, 12 valid bytes, reserved
    fd.extend([0x5A; 64]);
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&can).unwrap();
    let packed = z.finish().unwrap();
    let mut container = vec![2, 0, 0, 0, 0, 0, 0, 0];
    container.extend((can.len() as u32).to_le_bytes());
    container.extend([0; 4]);
    container.extend(&packed);

    let objects = [
        ("CanMessage2", lobj(32, 86, &can_body(8))),
        ("CanFdMessage64", lobj(32, 101, &fd64)), // 77 bytes, no padding
        ("Object", lobj(32, 999, &[7, 7])),       // 34 bytes + 2 padding
        ("CanMessage", can.clone()),
        ("LogContainer", lobj(16, 10, &container)),
        ("CanFdMessage", lobj(32, 100, &fd)),
        ("CanErrorExt", lobj(32, 73, &[0; 32])),
    ];
    for (_, o) in &objects {
        d.extend(o);
    }

    // (by magic) from 0, following "Next block"
    let mut at = 0u64;
    let mut seen = vec![];
    while at < d.len() as u64 {
        let n = apply(&t, &d, Root::ByMagic, at, &opts()).unwrap();
        check_no_errors(&n);
        seen.push((n.ty.clone(), at));
        at = n.next.as_ref().unwrap().offset;
    }
    assert_eq!(at, d.len() as u64);
    let mut expected = vec![("FileStatistics".to_string(), 0u64)];
    let mut off = 144;
    for (name, o) in &objects {
        expected.push((name.to_string(), off));
        off += o.len() as u64;
    }
    assert_eq!(seen, expected);

    let at = |name: &str| expected.iter().find(|e| e.0 == name).unwrap().1;
    let fs = apply(&t, &d, Root::ByMagic, 0, &opts()).unwrap();
    assert!(path(&fs, "statistics/api_number").value.starts_with("4070100"));
    let m = apply(&t, &d, Root::ByMagic, at("CanMessage2"), &opts()).unwrap();
    assert!(path(&m, "header/object_flags").value.contains("time_10us"), "{}", path(&m, "header/object_flags").value);
    assert!(path(&m, "body/flags").value.contains("tx"));
    assert_eq!(path(&m, "body/data").value, "01 02 03 04 05 06 07 08");
    let m = apply(&t, &d, Root::ByMagic, at("CanMessage"), &opts()).unwrap();
    assert!(path(&m, "header/original_timestamp").value.starts_with('0'));
    let m = apply(&t, &d, Root::ByMagic, at("CanFdMessage64"), &opts()).unwrap();
    assert_eq!(path(&m, "body/data").size, 5);
    let m = apply(&t, &d, Root::ByMagic, at("CanFdMessage"), &opts()).unwrap();
    assert_eq!(path(&m, "body/data").size, 12);
    assert!(path(&m, "body/fd_flags").value.contains("brs"));
    let m = apply(&t, &d, Root::ByMagic, at("Object"), &opts()).unwrap();
    assert_eq!(path(&m, "padding").size, 2);

    // the container's data decompresses into the objects
    let c = apply(&t, &d, Root::ByMagic, at("LogContainer"), &opts()).unwrap();
    assert!(path(&c, "body/compression").value.starts_with("zlib"));
    let data = path(&c, "body/data");
    let raw = &d[data.offset as usize..(data.offset + data.size) as usize];
    let inner = crate::compress::inflate(raw, None, 1 << 20).unwrap();
    assert_eq!(inner.data, can);
    let n = apply(&t, &inner.data, Root::ByMagic, 0, &opts()).unwrap();
    assert_eq!(n.ty, "CanMessage");
}

#[test]
fn sections_before_the_block_start() {
    // DT is preceded by an 8-byte recovery link that its length does not count
    let src = r###"#lang blocks 1
frame Plain(magic) align 8 {
    header { id: ascii(4) where id == magic, length: u32 }
    data: size(length - 8)
}
frame Recovered(magic) align 8 {
    recovery: [u64; 1] @ -8
    header { id: ascii(4) where id == magic, length: u32 }
    data: size(length - 8)
}
block HD: Plain("##HD") { data { first: u64 } }
block DT: Recovered("##DT") {
    recovery { link: -> HD | DT }
    data { records: [u8; ..] }
}
"###;
    let mut d = Vec::new();
    d.extend(b"##HD");
    d.extend(16u32.to_le_bytes());
    d.extend(24u64.to_le_bytes()); // HD at 0..16, DT's recovery link at 16, its ID at 24
    d.extend(0u64.to_le_bytes());
    d.extend(b"##DT");
    d.extend(12u32.to_le_bytes());
    d.extend(b"abcd");

    // a link points at the ID; the block includes the recovery link before it
    let dt = run(src, ty("DT | HD"), 24, &d);
    assert_eq!(dt.dispatched.as_deref(), Some("DT"));
    assert!(dt.error.is_none(), "{:?}", dt.error);
    assert_eq!((dt.offset, dt.size), (16, 20));
    let rec = child(&dt, "recovery");
    assert_eq!((rec.offset, rec.size), (16, 8));
    assert!(path(&dt, "recovery/link").link.is_none()); // 0 is null
    assert_eq!(child(&dt, "header").offset, 24);
    assert_eq!(path(&dt, "data/records").offset, 32);
    assert_eq!(dt.children.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["recovery", "header", "data"]);

    // "Next block" after HD is the recovery link; (by magic) finds the DT behind it
    let hd = run(src, ty("HD"), 0, &d);
    assert_eq!(hd.next.as_ref().unwrap().offset, 16);
    let m = run(src, Root::ByMagic, 16, &d);
    assert_eq!((m.ty.as_str(), m.offset), ("DT", 16));
    assert_eq!(child(&m, "header").offset, 24);
    let m = run(src, Root::ByMagic, 24, &d);
    assert_eq!((m.ty.as_str(), m.offset), ("DT", 16));

    // no room for the recovery link before the start of the file
    let at0 = run(src, ty("DT"), 0, &d);
    assert_eq!(child(&at0, "recovery").error.as_deref(), Some("offset -8 is before the start of the file"));

    // such sections must come first
    let e = err("#lang blocks 1\nframe F { header { id: u32 }\n back: [u64; 1] @ -8 }\nblock B: F { }\n");
    assert!(e.to_string().contains("must come before the other sections"), "{e}");
}
