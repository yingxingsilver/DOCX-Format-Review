fn main() {
    let s = "lineRule=exact line=400";
    println!("linerule={:?}", format_core::report::debug_kv(s));
    println!("line={:?}", format_core::report::debug_line(s));
    println!("plain={}", format_core::report::plain_term(s));
}
