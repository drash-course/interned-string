use istring::IString;

fn main() {
    let istring = IString::from("immutable and interned!");

    println!("Hello, {istring}");

    let another_one = IString::from("anothaone!");

    println!("And {another_one}");
}
