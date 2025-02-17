use std::{fmt::Debug, ops::Deref};
use storage::{IStringKey, ThreadLocalReader, SHARED_STORAGE, THREAD_LOCAL_READER};

mod storage;

/// An immutable and interned string.
/// 
/// Reading an `IString`'s contents is very fast, lock-free and wait-free.
/// It can be shared and read from any number of threads.
/// It scales linearly with the number of reading threads.
/// 
/// `IString` provides `Hash` and `Eq` implementations that run in O(1),
/// perfect for an high performance `HashMap<IString, _>`
/// 
/// The tradeoff is that creating a new `IString` is comparatively slower :
/// - Creating a new `IString` with a string that is already interned is fast and lock-free.
/// - Creating a new `IString` with a string that isn't already interned is slower.
///   It acquires a global lock and waits for all readers to finish reading.
#[derive(Eq, PartialEq, Ord, Hash)]
pub struct IString {
    pub(crate) key: IStringKey
}

// Indispensable traits impl : From, Drop, Deref

impl From<String> for IString {
    /// Intern the given `String` by consuming it. Its allocation is reused.
    /// 
    /// This operation runs in O(N) where N is the `string.len()`.
    /// If the string was already interned, this operation is lock-free.
    /// Otherwise, a global lock is acquired.
    /// 
    /// # Example
    /// 
    /// ```
    /// use interned_string::IString;
    /// 
    /// let my_istring = IString::from("hello".to_string());
    /// ```
    #[inline]
    fn from(string: String) -> Self {
        Self {
            // could block
            key: SHARED_STORAGE.insert_or_retain(string)
        }
    }
}

impl From<&str> for IString {
    /// Intern the given `&str` by cloning its contents.
    /// 
    /// This operation runs in O(N) where N is the `string.len()`.
    /// If the string was already interned, this operation is lock-free.
    /// Otherwise, a global lock is acquired.
    /// 
    /// # Example
    /// 
    /// ```
    /// use interned_string::IString;
    /// 
    /// let my_istring = IString::from("hello");
    /// ```
    #[inline]
    fn from(string: &str) -> Self {
        Self {
            // could block
            key: SHARED_STORAGE.insert_or_retain(String::from(string))
        }
    }
}

impl Drop for IString {
    #[inline]
    fn drop(&mut self) {
        THREAD_LOCAL_READER.with(|tl_reader| {
            tl_reader.release(self);
        });
    }
}

impl Deref for IString {
    type Target = str;
    
    /// Returns a reference to the string's contents.
    /// 
    /// This operation runs in O(1) and is lock-free.
    /// 
    /// # Example
    /// ```
    /// use interned_string::Intern;
    /// 
    /// fn foo(string: &str) {
    ///     println!("{string}")
    /// }
    /// 
    /// let my_istring = "hello".intern();
    /// // implicit call to Deref::deref
    /// foo(&my_istring);
    /// ```
    #[inline]
    fn deref(&self) -> &Self::Target {
        THREAD_LOCAL_READER.with(|reader: &ThreadLocalReader| {
            reader.read(self)
        })
    }
}

impl AsRef<str> for IString {
    /// Returns a reference to the string's contents.
    /// 
    /// This operation runs in O(1) and is lock-free.
    /// 
    /// # Example
    /// ```
    /// use interned_string::Intern;
    /// 
    /// let my_istring = "Hello, World!".intern();
    /// let (hello, world) = my_istring.as_ref().split_at(5);
    /// ```
    #[inline]
    fn as_ref(&self) -> &str {
        THREAD_LOCAL_READER.with(|tl_reader: &ThreadLocalReader| {
            tl_reader.read(self)
        })
    }
}

// Common traits impl that can't be derived : Clone, PartialOrd, Debug, Display, Default

impl Clone for IString {
    /// Returns a copy of the `IString`.
    /// 
    /// This operation runs in O(1) and is lock-free.
    #[inline]
    fn clone(&self) -> Self {
        THREAD_LOCAL_READER.with(|reader: &ThreadLocalReader| {
            reader.retain(self.key)
        });

        Self { key: self.key }
    }
}

impl PartialOrd for IString {
    #[inline]
    fn lt(&self, other: &Self) -> bool {
        self.deref().lt(other.deref())
    }

    #[inline]
    fn le(&self, other: &Self) -> bool {
        self.deref().le(other.deref())
    }

    #[inline]
    fn gt(&self, other: &Self) -> bool {
        self.deref().gt(other.deref())
    }

    #[inline]
    fn ge(&self, other: &Self) -> bool {
        self.deref().ge(other.deref())
    }
    
    #[inline]
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
    /// Creates an empty `IString`.
    #[inline]
    fn default() -> Self {
        Self::from(String::default())
    }
}

// Convenience trait Intern

pub trait Intern {
    fn intern(self) -> IString where Self: Sized;
}

impl Intern for String {
    /// Intern the given `String` by consuming it. Its allocation is reused.
    /// 
    /// This operation runs in O(N) where N is the `string.len()`.
    /// If the string was already interned, this operation is lock-free.
    /// Otherwise, a global lock is acquired.
    /// 
    /// # Example
    /// 
    /// ```
    /// use interned_string::Intern;
    /// 
    /// let my_istring = "hello".to_string().intern();
    /// ```
    #[inline]
    fn intern(self) -> IString {
        IString::from(self)
    }
}

impl Intern for &str {
    /// Intern the given `&str` by cloning its contents.
    /// 
    /// This operation runs in O(N) where N is the `string.len()`.
    /// If the string was already interned, this operation is lock-free.
    /// Otherwise, a global lock is acquired.
    /// 
    /// # Example
    /// 
    /// ```
    /// use interned_string::Intern;
    /// 
    /// let my_istring = "hello".intern();
    /// ```
    #[inline]
    fn intern(self) -> IString {
        IString::from(self)
    }
}

// Garbage collection

impl IString {
    /// Immediately frees all the interned strings that are no longer used.
    /// 
    /// Call this function when you wish to immediately reduce memory usage,
    /// at the cost of some CPU time. 
    /// This will acquire a global lock and wait for all readers to finish reading.
    /// It's recommended to only call this function when your program has nothing else to do.
    /// 
    /// Using this function is optional. Memory is always eventually freed.
    pub fn collect_garbage_now() {
        SHARED_STORAGE.writer.lock().unwrap().collect_garbage();
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
    fn it_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<IString>();
    }

    #[test]
    fn it_is_sync() {
        fn assert_sync<T: Sync>() {}
        assert_sync::<IString>();
    }

    #[cfg(feature = "serde")]
    #[test]
    fn it_serializes() {
        with_exclusive_use_of_shared_storage(|| {
            use serde::Serialize;

            #[derive(Serialize)]
            struct ExampleDTO {
                favorite_dish: IString
            }

            let dto = ExampleDTO { favorite_dish: "pasta".intern() };

            assert_eq!(serde_json::to_string(&dto).unwrap(), "{\"favorite_dish\":\"pasta\"}");
        });
    }

    #[cfg(feature = "serde")]
    #[test]
    fn it_deserializes() {
        with_exclusive_use_of_shared_storage(|| {
            use serde::Deserialize;

            #[derive(Deserialize, PartialEq, Debug)]
            struct ExampleDTO {
                favorite_dish: IString
            }

            let input = "{\"favorite_dish\":\"pasta\"}";

            let dto: Result<ExampleDTO, _> = serde_json::from_str(input);

            assert_eq!(dto.unwrap(), ExampleDTO { favorite_dish: "pasta".into() });
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
        writer.drain_channel_ops();
        writer.write_handle.append(storage::StringStorageOp::DropUnusedStrings);
        writer.write_handle.publish();
        drop(writer);
        drop(guard);
    }
}
