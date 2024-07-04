use std::{fmt::Debug, ops::Deref};
use storage::{IStringKey, ThreadLocalReader, SHARED_STORAGE, THREAD_LOCAL_READER};

mod storage;

/// An immutable and interned string.
/// 
/// Reading an `IString`'s contents is very fast, lock free and wait free (thanks to `left_right`).
/// Can be shared and read from any number of threads.
/// Scales linearly with the number of reading threads.
/// 
/// The tradeoff is that creating a new `IString` is comparatively slower :
/// - Creating a new `IString` with a string that is already interned is generally fast.
///   It acquires a global lock.
/// - Creating a new `IString` with a string that isn't already interned is much slower.
///   It acquired a global lock and waits for all readers to finish reading.
#[derive(Eq, PartialEq, Ord, Hash)]
pub struct IString {
    pub(crate) key: IStringKey
}

// Indispensable traits impl : From, Drop, Deref

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

impl AsRef<str> for IString {
    #[inline]
    fn as_ref(&self) -> &str {
        THREAD_LOCAL_READER.with(|reader: &ThreadLocalReader| {
            reader.read(self)
        })
    }
}

// Common traits impl that can't be derived : Clone, PartialOrd, Debug, Display, Default

impl Clone for IString {
    fn clone(&self) -> Self {
        SHARED_STORAGE.retain(self.key);

        Self { key: self.key }
    }
}

impl PartialOrd for IString {
    fn lt(&self, other: &Self) -> bool {
        self.deref().lt(other.deref())
    }

    fn le(&self, other: &Self) -> bool {
        self.deref().le(other.deref())
    }

    fn gt(&self, other: &Self) -> bool {
        self.deref().gt(other.deref())
    }

    fn ge(&self, other: &Self) -> bool {
        self.deref().ge(other.deref())
    }
    
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.deref().partial_cmp(other.deref())
    }
}

impl Debug for IString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("IString")
         .field(&self.deref())
         .finish()
    }
}

impl std::fmt::Display for IString {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

impl Default for IString {
    fn default() -> Self {
        Self::from(String::default())
    }
}

// Convenience trait Intern

pub trait Intern {
    fn intern(self) -> IString where Self: Sized;
}

impl Intern for String {
    #[inline]
    fn intern(self) -> IString {
        IString::from(self)
    }
}

impl Intern for &str {
    #[inline]
    fn intern(self) -> IString {
        IString::from(self)
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
            let my_istring1 = "hello".intern();
            assert!(my_istring1.deref() == "hello");

            assert_string_count_in_storage(1);
            assert_string_is_stored_with_key("hello", my_istring1.key);

            drop(my_istring1);

            assert_string_count_in_storage(1);
            assert_string_is_still_stored("hello");

            let my_istring2 = "another".to_string().intern();
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

    #[test]
    fn test_send() {
        fn assert_send<T: Send>() {}
        assert_send::<IString>();
    }

    #[test]
    fn test_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<IString>();
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
