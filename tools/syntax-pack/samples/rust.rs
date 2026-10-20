// Says hello.
use std::fmt;

struct Greeter {
    name: String,
}

impl fmt::Display for Greeter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "hello, {}", self.name)
    }
}

fn main() {
    let count = 42;
    println!("{} {count}", Greeter { name: "quark".into() });
}
