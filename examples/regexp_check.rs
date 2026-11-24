fn main() {
    let patterns: Vec<&str> = vec![
        "(?i:a)", "(?i-m:a)", "(?<a>x)(?<a>y)", "(?<a>x)", "a{2,1}", "[", "(",
        "\\1", "[b-a]", "\\u{110000}", "a**", "(?=a)*", "\\p{Foo}", "[\\w-a]",
        "(?<a>x)\\k<b>", "[]", "[^]", "\\c", "\\x", "\\u{", "a{1,2", "\\k<a>",
        "(?i)", "(?i:a)*", "(?--i:a)", "(?i-i:a)", "\\p{Script=Greek}", "[\\p{L}]",
        "\\-", "\\/", "\\8", "\\0", "\\00", "\\09", "\\400", "\\u{0}", "\\uD800",
        "\\u{D800}", "\\q", "\\e", "\\y", "\\B", "\\b", "\\Z", "\\A", "\\G",
        "\\K", "\\N", "\\R", "\\X", "\\h", "\\H", "\\v", "\\V", "\\o", "\\j",
        "\\l", "\\m", "\\n", "\\p", "\\s", "\\t", "\\w", "\\d", "\\D", "\\S",
        "\\W", "\\P", "\\f", "\\r", "\\a", "\\cA", "\\c1", "\\c_", "\\u{10FFFF}",
        "\\u{FFFFFFFF}", "\\u{1100011}", "(?i:a", "(?i:a))", "(?<a>", "(?<a>x)\\k<a>",
        "\\u{1F600}", "[\\d-a]", "[a-\\d]", "(?<1a>x)", "(?<a-b>x)", "(?<a>x)(?<b>y)",
    ];
    for p in patterns {
        let mut flags = v16::regexp::RegexFlags::default();
        flags.unicode = true;
        let r = v16::regexp::Regex::new_with_flags(p, flags);
        match r {
            Ok(_) => println!("OK   {:?}", p),
            Err(e) => println!("ERR  {:?} -> {}", p, e),
        }
    }
}
