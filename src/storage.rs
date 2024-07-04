use core::fmt;
use std::{
    collections::HashMap,
    mem::MaybeUninit,
    ops::Deref,
    sync::Mutex
};
use left_right::{Absorb, ReadHandle, WriteHandle};
use once_cell::sync::Lazy;
use radix_trie::{Trie, TrieKey};

use crate::IString;

pub(crate) type IStringKey = u32;

pub(crate) enum StringStorageOp {
    Insert { key: IStringKey, string: BoxedStr },
    Retain { key: IStringKey },
    // Note: releasing a string does not immediately free the storage, you have to run DropUnusedStrings as well.
    Release { key: IStringKey },
    DropUnusedStrings,
}

pub(crate) struct UniqueWriter {
    pub(crate) write_handle: WriteHandle<InnerStringStorage, StringStorageOp>,
    next_key: IStringKey,
}

// Needs to be Sync, so we need to use Mutex
pub(crate) struct ConcurrentStringStorage {
    pub(crate) writer: Mutex<UniqueWriter>,
    pub(crate) read_handle: Mutex<ReadHandle<InnerStringStorage>>,
}

impl ConcurrentStringStorage {
    fn new() -> Self {
        let (write_handle, read_handle) = left_right::new::<InnerStringStorage, StringStorageOp>();
        Self {
            writer: Mutex::new(UniqueWriter {
                write_handle: write_handle,
                next_key: 0,
            }),
            read_handle: Mutex::new(read_handle),
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
        let mut writer = self.writer.lock().unwrap();
        let key = writer.next_key;
        // TODO: scan the storage for reusable keys when it overflows, instead of panic'ing
        writer.next_key = writer.next_key.checked_add(1).unwrap();
        writer.write_handle.append(StringStorageOp::Insert { key, string });
        writer.write_handle.append(StringStorageOp::DropUnusedStrings);
        writer.write_handle.publish();
        return key;
    }

    #[inline]
    fn retain(&self, key: IStringKey) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_handle.append(StringStorageOp::Retain { key });
        // optimisation: do not publish here
    }

    #[inline]
    pub(crate) fn release(&self, istring: &mut IString) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_handle.append(StringStorageOp::Release { key: istring.key });
        // optimisation: do not publish here
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

impl StoredString {
    fn new(string: BoxedStr) -> Self {
        Self { inner: string, strong_count: 1 }
    }

    #[inline]
    fn retain(&mut self) {
        self.strong_count += 1;
    }

    #[inline]
    fn release(&mut self) {
        self.strong_count = self.strong_count.checked_sub(1).unwrap();
    }

    #[inline]
    fn is_droppable(&self) -> bool {
        self.strong_count == 0
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

    fn clone_with_aliasing(&mut self) -> Self {
        // Safety: this is ok because the contents are always init,
        // and thanks to MaybeUninit<_> the compiler can't assume noalias
        // so it's fine to copy the box (the fat pointer) to make a new BoxedStr.
        Self {
            contents: MaybeUninit::new(unsafe { self.contents.assume_init_read() })
        }
    }

    unsafe fn free(self) {
        // Calling free() on a BoxedStr that is still being aliased will cause a double free.
        // The caller must make sure that `self` is the last BoxedStr that is sharing (aliasing) the contents.
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
    pub(crate) strings_to_possibly_free: Vec<IStringKey>,
}

impl Default for InnerStringStorage {
    fn default() -> Self {
        Self {
            trie: Trie::new(),
            map: HashMap::new(),
            strings_to_possibly_free: Vec::new()
        }
    }
}

impl InnerStringStorage {
    #[inline]
    fn retain(&mut self, key: IStringKey) {
        let stored_string = self.map.get_mut(&key).unwrap();
        stored_string.retain();
    }

    #[inline]
    fn release(&mut self, key: IStringKey) {
        let stored_string = self.map.get_mut(&key).unwrap();
        stored_string.release();
        if stored_string.is_droppable() {
            self.strings_to_possibly_free.push(key);
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

                let stored_string_with_aliasing = StoredString::new(string.clone_with_aliasing());

                let previous_stored = self.map.insert(*key, stored_string_with_aliasing);
                debug_assert!(
                    previous_stored.is_none(),
                    "Inserting a new string '{}' in map but a value is already set for key {}", string, *key
                );
            },
            StringStorageOp::Retain { key } => self.retain(*key),
            StringStorageOp::Release { key } => self.release(*key),
            StringStorageOp::DropUnusedStrings => {
                // Note:
                // Since we are in absorb_first, we cant free() the unused `BoxedStr`s because
                // they are still being aliased by the read map's and the write map's `StoredString`s
                for string_key in self.strings_to_possibly_free.drain(..) {
                    let stored = self.map.remove(&string_key).unwrap();
                    // make sure that the string is actually unused
                    if stored.is_droppable() {
                        // remove it from the trie as well
                        let removed_key = self.trie.remove(&stored.inner);
                        debug_assert!(removed_key == Some(string_key));

                        // Note: we can't free() the BoxedStr here because it's still being aliased
                        // by the other map. We just drop it, which essentially does a forget()
                        drop(stored)
                    } else {
                        // put the StoredString back in the map.
                        // we optimise for the "if" branch, so in this "else" branch we do more work: remove + insert.
                        // otherwise, the "if" branch would have to do get + remove, instead of just remove.
                        self.map.insert(string_key, stored);
                    }
                }
            }
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
            StringStorageOp::Release { key } => self.release(key),
            StringStorageOp::DropUnusedStrings => {
                for string_key in self.strings_to_possibly_free.drain(..) {
                    let stored = self.map.remove(&string_key).unwrap();
                    // make sure that the string is actually unused
                    if stored.is_droppable() {
                        // remove it from the trie as well
                        let removed_key = self.trie.remove(&stored.inner);
                        debug_assert!(removed_key == Some(string_key));

                        // Safety:
                        // Since we are in absorb_second, we can free() the BoxedStr because it's now uniquely
                        // referenced by the write map's StoredString, because absorbed_first already ran for the given
                        // operation, and must have dropped the other BoxedStr.
                        unsafe { stored.inner.free() };
                    } else {
                        // put the StoredString back in the map.
                        // we optimise for the "if" branch, so in this "else" branch we do more work: remove + insert.
                        // otherwise, the "if" branch would have to do get + remove, instead of just remove.
                        self.map.insert(string_key, stored);
                    }
                }
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
