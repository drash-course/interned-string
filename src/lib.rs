use std::{fmt::Debug, ops::Deref};
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

impl Debug for IString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("IString")
         .field(&self.deref())
         .finish()
    }
}

#[cfg(feature = "serde")]
mod feature_serde {
    use serde::{de::Visitor, Deserialize, Serialize};
    use crate::IString;

    impl Serialize for IString {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            serializer.serialize_str(std::ops::Deref::deref(&self))
        }
    }
    
    impl<'de> Deserialize<'de> for IString {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_string(IStringVisitor)
        }
    }
    
    struct IStringVisitor;
    
    impl<'de> Visitor<'de> for IStringVisitor {
        type Value = IString;
    
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a string")
        }
    
        fn visit_string<E: serde::de::Error>(self, string: String) -> Result<Self::Value, E> {
            // does not need to allocate a new string
            Ok(IString::from(string))
        }
    
        fn visit_str<E: serde::de::Error>(self, slice: &str) -> Result<Self::Value, E> {
            // less performant, will allocate
            Ok(IString::from(slice))
        }
    }
}

// tests

#[cfg(test)]
mod tests {
    use std::{ops::Deref, sync::Mutex};
    use radix_trie::TrieCommon;

    use super::*;
    use crate::storage::SHARED_STORAGE;

    #[test]
    fn it_creates_and_removes_1_string() {
        with_exclusive_use_of_shared_storage(|| {
            let my_istring1 = IString::from("hello");
            assert!(my_istring1.deref() == "hello");

            assert_string_count_in_storage(1);
            assert_string_is_stored_with_key("hello", my_istring1.key);

            drop(my_istring1);

            assert_string_count_in_storage(1);
            assert_string_is_still_stored("hello");

            let my_istring2 = IString::from("another");
            assert!(my_istring2.deref() == "another");

            assert_string_count_in_storage(1);
            assert_string_is_stored_with_key("another", my_istring2.key);
            assert_string_is_not_stored("hello")
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

            assert_string_count_in_storage(1);
            assert_string_is_stored_with_key("hello", my_istring1.key);

            drop(my_istring1);

            assert_string_count_in_storage(1);
            assert_string_is_stored_with_key("hello", my_istring2.key);

            drop(my_istring2);

            assert_string_count_in_storage(1);
            assert_string_is_still_stored("hello");
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

            assert_string_count_in_storage(3);
            assert_string_is_stored_with_key("hello", my_istring1.key);
            assert_string_is_stored_with_key("world", my_istring2.key);
            assert_string_is_stored_with_key("howdy", my_istring3.key);
            assert_string_is_not_stored("hola");

            drop(my_istring1);
            drop(my_istring2);

            assert_string_count_in_storage(3);
            assert_string_is_still_stored("hello");
            assert_string_is_still_stored("world");
            assert_string_is_stored_with_key("howdy", my_istring3.key);
            assert_string_is_not_stored("hola");

            // it should reuse the storage
            let my_istring1bis = IString::from("hello");
            assert!(my_istring1bis.deref() == "hello");

            // and not clean up the storage of "world" yet
            assert_string_count_in_storage(3);
            assert_string_is_stored_with_key("hello", my_istring1bis.key);
            assert_string_is_stored_with_key("howdy", my_istring3.key);
            assert_string_is_still_stored("world");

            let my_istring4 = IString::from("another");
            assert!(my_istring4.deref() == "another");

            // creating a new string should cause the storage of unused strings to be cleaned up
            assert_string_is_stored_with_key("hello", my_istring1bis.key);
            assert_string_is_stored_with_key("howdy", my_istring3.key);
            assert_string_is_stored_with_key("another", my_istring4.key);
            assert_string_is_not_stored("world");
            assert_string_count_in_storage(3);
        });
    }

    fn assert_string_count_in_storage(count: usize) {
        let guard = SHARED_STORAGE.read_handle.lock().unwrap();
        let read_handle = guard.enter().unwrap();
        assert_eq!(read_handle.map.len(), count);
        assert_eq!(read_handle.trie.len(), count);
    }

    fn assert_string_is_still_stored(string: &str) {
        let guard = SHARED_STORAGE.read_handle.lock().unwrap();
        let read_handle = guard.enter().unwrap();
        let key = read_handle.trie.get(&string.into());
        if let Some(key) = key {
            assert!(read_handle.map.get(&key).unwrap().inner.deref() == string);
        } else {
            assert!(false, "the string is not in the trie");
        }
    }

    fn assert_string_is_stored_with_key(string: &str, key: u32) {
        let guard = SHARED_STORAGE.read_handle.lock().unwrap();
        let read_handle = guard.enter().unwrap();
        assert!(read_handle.map.get(&key).unwrap().inner.deref() == string);
        assert_eq!(read_handle.trie.get(&string.into()), Some(&key));
    }

    fn assert_string_is_not_stored(string: &str) {
        let guard = SHARED_STORAGE.read_handle.lock().unwrap();
        let read_handle = guard.enter().unwrap();
        assert_eq!(read_handle.trie.get(&string.into()), None);
    }

    static SHARED_STORAGE_MUTEX: Mutex<()> = Mutex::new(());

    fn with_exclusive_use_of_shared_storage(closure: fn()) {
        let guard = SHARED_STORAGE_MUTEX.lock().expect("test lock is not poisoned");
        closure();

        // reset the writer for the next test
        let mut writer = SHARED_STORAGE.writer.lock().unwrap();
        writer.write_handle.append(storage::StringStorageOp::DropUnusedStrings);
        writer.write_handle.publish();
        drop(writer);
        drop(guard);
    }
}
