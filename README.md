## interned-string

This crate exposes `IString`, an 
[immutable and interned string](https://en.wikipedia.org/wiki/String_interning) type.

It's built for high performance and with multithreading in mind.

It provides O(1) `Hash` and `Eq` operations, perfect for your `HashMap<IString, _>`.

## Getting Started

You can intern any `String` or `&str` value by calling `intern()`.

You can pass an `IString` by reference to any function that accepts a `&str`.

```rust
use interned_string::Intern;

fn main() {
    let my_istring = "hello".intern();

    foo(&my_istring);
}

fn foo(string: &str) {
    println!("{string}");
}
```

If you enable the `serde` feature, you can use `IString` in place of `String` in your DTOs.

```toml
[dependencies]
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
interned-string = { version = "0.1", features = ["serde"] }
```

```rust
use serde::Deserialize;
use interned_string::IString;

#[derive(Deserialize, Debug)]
struct ExampleDTO {
    favorite_dish: IString
}

fn main() {
    let serialized = "{\"favorite_dish\":\"pasta\"}";

    let example: ExampleDTO = serde_json::from_str(&serialized).unwrap();

    println!("{example:?}")
    // ExampleDTO { favorite_dish: IString("pasta") }
}

```

## Performance Characteristics

Reading an `IString`'s contents is very fast, lock free and wait free (thanks to `left_right`).
The `IString` can be shared and read from any number of threads.
It scales linearly with the number of reading threads.

The tradeoff is that creating a new `IString` is slower.
A radix tree (compact trie) needs to be traversed to deduplicate the new string.
If the string wasn't interned yet, a lock needs to be acquired, the tree needs to be updated,
and string needs to be inserted in storage.
While the tree walk can be done in parallel from multiple threads, the lock prevents linear 
scaling for writes.

## Planned Improvements

- Replace or rewrite the radix tree to make it reuse the string storage, instead of storing a clone
  of the each interned string.
  Currently the crate uses 2x the interned string storage space because of this (1x in storage,
  1x as a clone in the radix tree).

## Contributing

Feel free to open a PR. Any contribution is **greatly appreciated**.

If you have a suggestion, please open an Issue.

## License

[Mozilla Public License 2.0](https://www.mozilla.org/en-US/MPL/2.0/)

## Acknowledgments

Special thanks to Jon Gjengset (@jonhoo) for providing the inspiration for this crate, and for his
work on `left-right`.
