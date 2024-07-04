use std::ops::Deref;
use storage::{IStringKey, ThreadLocalReader, SHARED_STORAGE, THREAD_LOCAL_READER};

mod storage;

/// An immutable and interned string.
/// 
/// Reading an `IString`'s contents is very fast, lock free and wait free thanks to the `left_right` crate.
/// Can be shared and read from any number of threads.
/// Scales linearly with the number of reading threads.
/// 
/// The tradeoff being that creating a new `IString` is much slower.
/// A radix tree (compact trie) needs to be traversed to deduplicate the string,
/// a lock needs to be acquired, and the tree needs to be updated in case of a new string.
/// While the tree walk can be done in parallel from multiple threads,
/// the lock prevents linear scaling for writes.
/// 
/// Dropping an `IString` also acquires a lock.
pub struct IString {
    pub(crate) key: IStringKey
}

impl From<String> for IString {
    #[inline]
    fn from(string: String) -> Self {
        Self {
            key: SHARED_STORAGE.insert_or_retain(string)
        }
    }
}

impl From<&str> for IString {
    #[inline]
    fn from(string: &str) -> Self {
        Self {
            key: SHARED_STORAGE.insert_or_retain(String::from(string))
        }
    }
}

impl Drop for IString {
    #[inline]
    fn drop(&mut self) {
        SHARED_STORAGE.release(self)
    }
}

impl Deref for IString {
    type Target = str;
    
    #[inline]
    fn deref(&self) -> &Self::Target {
        THREAD_LOCAL_READER.with(|reader: &ThreadLocalReader| {
            reader.read(self)
        })
    }
}

impl std::fmt::Display for IString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

#[cfg(test)]
mod tests {
    use std::{ops::Deref, sync::Mutex};
    use radix_trie::TrieCommon;
    use storage::BoxStr;

    use super::*;
    use crate::storage::SHARED_STORAGE;

    #[test]
    fn it_creates_and_removes_1_string() {
        with_exclusive_use_of_shared_storage(|| {
            let my_istring = IString::from("hello");
            assert!(my_istring.deref() == "hello");

            {
                let guard = SHARED_STORAGE.read_handle.lock().unwrap();
                let read_handle = guard.enter().unwrap();
                assert!(read_handle.map.len() == 1);
                assert!(read_handle.map.get(&my_istring.key).unwrap().inner.contents.deref() == "hello");
                assert!(read_handle.trie.len() == 1);
                assert!(read_handle.trie.get(&BoxStr { contents: "hello".to_string().into_boxed_str() }) == Some(&my_istring.key));
            }

            drop(my_istring);

            {
                let guard = SHARED_STORAGE.read_handle.lock().unwrap();
                let read_handle = guard.enter().unwrap();
                assert!(read_handle.map.len() == 0);
                assert!(read_handle.trie.len() == 0);
            }
        });
    }

    #[test]
    fn it_creates_and_removes_1_shared_string() {
        with_exclusive_use_of_shared_storage(|| {
            let my_istring1 = IString::from("hello");
            let my_istring2 = IString::from("hello");
            assert!(my_istring1.deref() == "hello");
            assert!(my_istring2.deref() == "hello");
            assert!(my_istring1.key == my_istring2.key);

            {
                let guard = SHARED_STORAGE.read_handle.lock().unwrap();
                let read_handle = guard.enter().unwrap();
                assert!(read_handle.map.len() == 1);
                assert!(read_handle.map.get(&my_istring1.key).unwrap().inner.contents.deref() == "hello");
                assert!(read_handle.trie.len() == 1);
                assert!(read_handle.trie.get(&BoxStr { contents: "hello".to_string().into_boxed_str() }) == Some(&my_istring1.key));
                assert!(read_handle.trie.get(&BoxStr { contents: "hola".to_string().into_boxed_str() }) == None);
            }

            drop(my_istring1);

            {
                let guard = SHARED_STORAGE.read_handle.lock().unwrap();
                let read_handle = guard.enter().unwrap();
                assert!(read_handle.map.len() == 1);
                assert!(read_handle.map.get(&my_istring2.key).unwrap().inner.contents.deref() == "hello");
                assert!(read_handle.trie.len() == 1);
                assert!(read_handle.trie.get(&BoxStr { contents: "hello".to_string().into_boxed_str() }) == Some(&my_istring2.key));
                assert!(read_handle.trie.get(&BoxStr { contents: "hola".to_string().into_boxed_str() }) == None);
            }

            drop(my_istring2);

            {
                let guard = SHARED_STORAGE.read_handle.lock().unwrap();
                let read_handle = guard.enter().unwrap();
                assert!(read_handle.map.len() == 0);
                assert!(read_handle.trie.len() == 0);
            }
        });
    }

    #[test]
    fn it_creates_and_removes_3_strings() {
        with_exclusive_use_of_shared_storage(|| {
            let my_istring1 = IString::from("hello");
            let my_istring2 = IString::from("world");
            let my_istring3 = IString::from("howdy");
            assert!(my_istring1.deref() == "hello");
            assert!(my_istring2.deref() == "world");
            assert!(my_istring3.deref() == "howdy");
            assert!(my_istring1.key != my_istring2.key);
            assert!(my_istring2.key != my_istring3.key);

            {
                let guard = SHARED_STORAGE.read_handle.lock().unwrap();
                let read_handle = guard.enter().unwrap();
                assert!(read_handle.map.len() == 3);
                assert!(read_handle.map.get(&my_istring1.key).unwrap().inner.contents.deref() == "hello");
                assert!(read_handle.map.get(&my_istring2.key).unwrap().inner.contents.deref() == "world");
                assert!(read_handle.map.get(&my_istring3.key).unwrap().inner.contents.deref() == "howdy");

                assert!(read_handle.trie.len() == 3);
                assert!(read_handle.trie.get(&BoxStr { contents: "hello".to_string().into_boxed_str() }) == Some(&my_istring1.key));
                assert!(read_handle.trie.get(&BoxStr { contents: "world".to_string().into_boxed_str() }) == Some(&my_istring2.key));
                assert!(read_handle.trie.get(&BoxStr { contents: "howdy".to_string().into_boxed_str() }) == Some(&my_istring3.key));
                assert!(read_handle.trie.get(&BoxStr { contents: "hola".to_string().into_boxed_str() }) == None);
            }

            drop(my_istring1);
            drop(my_istring3);

            {
                let guard = SHARED_STORAGE.read_handle.lock().unwrap();
                let read_handle = guard.enter().unwrap();
                assert!(read_handle.map.len() == 1);
                assert!(read_handle.map.get(&my_istring2.key).unwrap().inner.contents.deref() == "world");

                assert!(read_handle.trie.len() == 1);
                assert!(read_handle.trie.get(&BoxStr { contents: "hello".to_string().into_boxed_str() }) == None);
                assert!(read_handle.trie.get(&BoxStr { contents: "world".to_string().into_boxed_str() }) == Some(&my_istring2.key));
                assert!(read_handle.trie.get(&BoxStr { contents: "howdy".to_string().into_boxed_str() }) == None);
                assert!(read_handle.trie.get(&BoxStr { contents: "hola".to_string().into_boxed_str() }) == None);
            }

            drop(my_istring2);

            {
                let guard = SHARED_STORAGE.read_handle.lock().unwrap();
                let read_handle = guard.enter().unwrap();
                assert!(read_handle.map.len() == 0);
                assert!(read_handle.trie.len() == 0);
            }
        });
    }

    static SHARED_STORAGE_MUTEX: Mutex<()> = Mutex::new(());

    fn with_exclusive_use_of_shared_storage(closure: fn()) {
        let guard = SHARED_STORAGE_MUTEX.lock();
        closure();
        drop(guard);
    }
}
