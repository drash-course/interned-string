use std::ops::Deref;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use interned_string::IString;

fn a_bigger_bunch(c: &mut Criterion) {
    c.bench_function(
        "creating a bigger bunch of IStrings", 
        |bencher| {
            bencher.iter(|| {
                let my_istring1 = IString::from(black_box("A short string that the compiler can't inline"));
                let my_istring2 = IString::from(black_box("Another one"));
                let my_istring3 = IString::from(black_box("A very long string that could come for example from the network, or is read from a file, or something else like that. Basically something that a user of the libray may want to intern without having to think about it too much"));
                let my_istring4 = IString::from(black_box("Another one"));
                let my_istring5 = IString::from(black_box("A very long string that could come for example from the network, or is read from a file, or something else like that, but is not like the other long string. Sike!"));


                if my_istring3.deref() == my_istring5.deref() {
                    black_box("something useless that the compiler can't remove");
                }

                drop(my_istring5);

                if my_istring1.deref() == my_istring4.deref() || *my_istring2 == *my_istring4 {
                    black_box("something useless that the compiler can't remove");
                }
            })
        }
    );
}

criterion_group!(
    name = benches;
    config = Criterion::default();
    targets = a_bigger_bunch
);
criterion_main!(benches);
