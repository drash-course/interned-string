use core::fmt;
use std::{
    borrow::Borrow,
    collections::HashMap,
    mem::MaybeUninit,
    ops::Deref,
    sync::{atomic::{AtomicU32, Ordering}, Mutex}
};
use left_right::{Absorb, ReadHandle, WriteHandle};
use once_cell::sync::Lazy;
use radix_trie::{Trie, TrieKey};

use crate::IString;

pub(crate) type IStringKey = u32;

enum StringStorageOp {
    Insert { key: IStringKey, string: BoxedStr },
    Retain { key: IStringKey },
    Release { key: IStringKey }
}

// Needs to be Sync, so we need to use Mutex
pub(crate) struct ConcurrentStringStorage {
    write_handle: Mutex<WriteHandle<InnerStringStorage, StringStorageOp>>,
    pub(crate) read_handle: Mutex<ReadHandle<InnerStringStorage>>,
    next_key: AtomicU32
}

impl ConcurrentStringStorage {
    fn new() -> Self {
        let (write, read) = left_right::new::<InnerStringStorage, StringStorageOp>();
        Self {
            write_handle: Mutex::new(write),
            read_handle: Mutex::new(read),
            next_key: 0.into(),
        }
    }

    #[inline]
    pub(crate) fn insert_or_retain(&self, string: String) -> IStringKey {
        let boxed: BoxedStr = string.into();
        let found_key: Option<IStringKey> = THREAD_LOCAL_READER.with(|reader: &ThreadLocalReader| {
            let storage = reader.read_handle.enter().expect("reader is available");
            return storage.trie.get(&boxed).copied();
        });

        if let Some(key) = found_key {
            // string is already in storage
            self.retain(key);
            return key;
        } else {
            // string is not in storage yet
            return self.insert(boxed);
        }
    }

    #[inline]
    fn insert(&self, string: BoxedStr) -> IStringKey {
        let key = self.next_key.fetch_add(1, Ordering::SeqCst);
        let mut writer = self.write_handle.lock().unwrap();
        writer.append(StringStorageOp::Insert { key, string });
        writer.publish();
        return key;
    }

    #[inline]
    fn retain(&self, key: IStringKey) {
        let mut writer = self.write_handle.lock().unwrap();
        writer.append(StringStorageOp::Retain { key });
        writer.publish();
    }

    #[inline]
    pub(crate) fn release(&self, istring: &mut IString) {
        let mut writer = self.write_handle.lock().unwrap();
        writer.append(StringStorageOp::Release { key: istring.key });
        writer.publish();
    }
}

// does not need to be Sync nor Send :-)
pub(crate) struct ThreadLocalReader {
    read_handle: ReadHandle<InnerStringStorage>
}

impl ThreadLocalReader {
    fn from(css: &ConcurrentStringStorage) -> Self {
        Self {
            read_handle: css.read_handle.lock().unwrap().clone(),
        }
    }

    #[inline]
    pub(crate) fn read<'a>(&self, istring: &'a IString) -> &'a str {
        let iss = self.read_handle.enter().expect("reader is available");
        let stored_string = iss.map.get(&istring.key).expect("a valid IString implies that the storage has it's string contents");
        // Safety: we hold a reference to an IString that lives for 'a
        //         so the IString won't be dropped for at least 'a
        //         so the BoxedString we get from storage must live for at least 'a as well.
        return unsafe { stored_string.inner.get() }
    }
}

#[derive(Clone)]
pub(crate) struct StoredString {
    pub(crate) inner: BoxedStr,
    strong_count: usize
}

enum StoredStringReleaseResult {
    IsDroppable, IsReferenced
}

impl StoredString {
    fn new(string: BoxedStr) -> Self {
        Self { inner: string, strong_count: 1 }
    }

    #[inline]
    fn retain(&mut self) {
        self.strong_count += 1;
    }

    #[inline(always)] // to optimize the enum away
    fn release(&mut self) -> StoredStringReleaseResult {
        self.strong_count -= 1;
        if self.strong_count == 0 {
            return StoredStringReleaseResult::IsDroppable
        } else {
            return StoredStringReleaseResult::IsReferenced
        };
    }
}

/// A wrapper type around a `Box<str>` that provides facilities to
/// unsafely clone it with pointer aliasing to save memory.
pub(crate) struct BoxedStr {
    contents: MaybeUninit<Box<str>>
}

impl PartialEq for BoxedStr {
    fn eq(&self, other: &Self) -> bool {
        self.get_contents() == other.get_contents()
    }
}

impl Eq for BoxedStr {}

impl Clone for BoxedStr {
    fn clone(&self) -> Self {
        Self { contents: MaybeUninit::new(self.get_contents().clone()) }
    }
}

impl BoxedStr {
    #[inline]
    fn get_contents(&self) -> &Box<str> {
        // Safety: the contents are always init.
        // MaybeUninit<...> is only used to disallow the compiler to assume noalias.
        unsafe { self.contents.assume_init_ref() }
    }

    unsafe fn clone_with_aliasing(&mut self) -> Self {
        Self {
            contents: MaybeUninit::new(self.contents.assume_init_read())
        }
    }

    unsafe fn free(self) {
        let contents = self.contents.assume_init();
        drop(contents);
    }

    unsafe fn get<'a>(&self) -> &'a str {
        let slice: &str = &self.get_contents().as_ref();
        // Safety: this extends the lifetime of `slice` from 'self (the lifetime of the borrowed self)
        // to an arbitrary 'a that the caller chooses.
        // This is unsafe because the caller must manually choose a lifetime that actually does not
        // exceed the lifetime of the `BoxedStr`.
        std::mem::transmute(slice)
    }
}

impl Deref for BoxedStr {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.get_contents().as_ref()
    }
}

impl fmt::Display for BoxedStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.deref())
    }
}

impl From<String> for BoxedStr {
    fn from(value: String) -> Self {
        Self { contents: MaybeUninit::new(value.into_boxed_str()) }
    }
}

#[cfg(test)]
impl From<&str> for BoxedStr {
    fn from(value: &str) -> Self {
        Self { contents: MaybeUninit::new(value.to_string().into_boxed_str()) }
    }
}

impl TrieKey for BoxedStr {
    #[inline]
    fn encode_bytes(&self) -> Vec<u8> {
        self.get_contents().encode_bytes()
    }
}

pub(crate) struct InnerStringStorage {
    pub(crate) trie: Trie<BoxedStr, IStringKey>,
    pub(crate) map: HashMap<IStringKey, StoredString>,
}

impl Default for InnerStringStorage {
    fn default() -> Self {
        Self {
            trie: Trie::new(),
            map: HashMap::new(),
        }
    }
}

impl InnerStringStorage {
    #[inline]
    fn retain(&mut self, key: IStringKey) {
        let stored_string = self.map.get_mut(&key).unwrap();
        stored_string.retain();
    }

    #[inline] // optimize away the bool
    fn release(&mut self, key: IStringKey, allowed_to_free_BoxedStr: bool) {
        let stored_string = self.map.get_mut(&key).unwrap();
        match stored_string.release() {
            StoredStringReleaseResult::IsDroppable => {
                let owned_stored_string = self.map.remove(&key).unwrap();
                let removed_key = self.trie.remove(owned_stored_string.inner.borrow());
                debug_assert!(
                    removed_key.is_some(),
                    "Removed string '{}' from trie but it was not found", owned_stored_string.inner
                );
                debug_assert!(
                    removed_key.unwrap() == key,
                    "The string '{}' that was removed from the trie does not match the key", owned_stored_string.inner
                );
                if allowed_to_free_BoxedStr {
                    // Safety (from caller):
                    // Since we are in absorb_first, we cant free() the BoxedStr contents because
                    // it's still being aliased by the read map's StoredString (1) and the write map's StoredString (2).
                    // Dropping __any__ one of the two now would create a dangling pointer in the other.
                    unsafe { owned_stored_string.inner.free() }
                }
            },
            StoredStringReleaseResult::IsReferenced => {
                // do nothing else
            },
        }
    }
}

impl Absorb<StringStorageOp> for InnerStringStorage {
    fn absorb_first(&mut self, operation: &mut StringStorageOp, _other: &Self) {
        match operation {
            StringStorageOp::Insert { key, string } => {
                let previous_key = self.trie.insert(string.clone(), *key);
                debug_assert!(
                    previous_key.is_none(),
                    "Inserting a new string '{}' in tree but there is already a key {} for it ", string, previous_key.unwrap()
                );

                // Safety:
                // The BoxedStr contents is now being aliased from stored_string_with_aliasing (1) and operation (2).
                // Dropping __any__ one of the two now would create a dangling pointer in the other.
                // This is fine because (1) will be inserted into the map and won't be dropped,
                // and (2) will be passed into `absorb_second` and won't be dropped until then either.
                let stored_string_with_aliasing = StoredString::new(unsafe { string.clone_with_aliasing() });

                let previous_stored = self.map.insert(*key, stored_string_with_aliasing);
                debug_assert!(
                    previous_stored.is_none(),
                    "Inserting a new string '{}' in map but a value is already set for key {}", string, *key
                );
            },
            StringStorageOp::Retain { key } => self.retain(*key),
            StringStorageOp::Release { key } => {
                // Safety:
                // Since we are in absorb_first, we cant free() the BoxedStr contents because
                // it's still being aliased by the read map's StoredString (1) and the write map's StoredString (2).
                // Dropping __any__ one of the two now would create a dangling pointer in the other.
                self.release(*key, false)
            },
        }
    }

    fn absorb_second(&mut self, operation: StringStorageOp, _other: &Self) {
        match operation {
            StringStorageOp::Insert { key, string } => {
                let previous_key = self.trie.insert(string.clone(), key);
                debug_assert!(
                    previous_key.is_none(),
                    "Inserting a new string '{}' in tree but there is already a key {} for it ", &string, previous_key.unwrap()
                );

                let previous_stored = self.map.insert(key, StoredString::new(string));
                debug_assert!(
                    previous_stored.is_none(),
                    "Inserting a new string '{}' in map but an older string '{}' was already set for key {}",
                    &self.map.get(&key).unwrap().inner,
                    previous_stored.unwrap().inner,
                    key
                );
            },
            StringStorageOp::Retain { key } => self.retain(key),
            StringStorageOp::Release { key } => {
                // Safety:
                // Since we are in absorb_second, we can free() the BoxedStr contents because it's now uniquely
                // referenced by the write map's StoredString, because absorbed_first already ran for the given
                // operation, and must have manually dropped the BoxedStr inside the StoredString.
                self.release(key, true);
            },
        }
    }

    fn sync_with(&mut self, first: &Self) {
        self.trie = first.trie.clone();
        self.map = first.map.clone();
    }
}

pub(crate) static SHARED_STORAGE: Lazy<ConcurrentStringStorage> = Lazy::new(|| {
    ConcurrentStringStorage::new()
});

thread_local! {
    pub(crate) static THREAD_LOCAL_READER: ThreadLocalReader = ThreadLocalReader::from(&SHARED_STORAGE);
}
