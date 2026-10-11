use format_core::report::{debug_kv, debug_line, plain_term};

fn main() {
    for s in [
        "lineRule=exact line=400",
        "lineRule= atLeast line=300",
        "w:lineRule=exact w:line=400",
        "lineRule=exact",
        "before-spacing lineRule=exact line=400",
    ] {
        println!("{s:?}\n  kv={:?} num={:?} term={:?}", debug_kv(s), debug_line(s), plain_term(s));
    }
}
