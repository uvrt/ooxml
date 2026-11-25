//! Owns: a saved file's MS-FSSHTTPD chunks as Python reads them (`ooxml_core.chunk_file`,
//! `FileChunks`, `FileChunk`, `ChunkDelta`): what a co-authoring host (the WOPI host) stores
//! of a version, and which chunks of a new version it must be sent. The chunking is
//! ooxml-zip's (`ooxml_zip::chunks`), with random signatures from the operating system's
//! random source.
//!
//! Must not: hold logic beyond conversion, or keep a Python object.

use ooxml_zip::chunks::{Chunk, Chunking, Delta, Leaf, OsRandom, Reuse, Signature};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyBytes;
use std::sync::Arc;

/// A leaf as Python holds it: `(signature, size)`.
type PyLeaf<'py> = (Bound<'py, PyBytes>, u64);

fn leaf<'py>(py: Python<'py>, leaf: Leaf) -> PyLeaf<'py> {
    (PyBytes::new(py, leaf.signature.as_bytes()), leaf.size)
}

/// `data` cut into MS-FSSHTTPD 2.4.1 chunks and signed as Office signs them: an entry of at
/// most 4,096 bytes one chunk, a larger one a header and a data chunk, the central directory
/// the final chunk; a chunk over 1 MiB with random subchunks. A file Office would not
/// analyse as ZIP is cut into 1 MiB pieces signed with their SHA-1.
#[pyfunction]
pub(crate) fn chunk_file(py: Python<'_>, data: &[u8]) -> PyFileChunks {
    let data = data.to_owned();
    let chunking = py.detach(move || Chunking::of(&data, &mut OsRandom::new()));
    PyFileChunks {
        inner: Arc::new(chunking),
    }
}

/// A file's chunks, in file order (`ooxml_core.chunk_file`).
#[pyclass(frozen, name = "FileChunks", module = "ooxml_core")]
pub struct PyFileChunks {
    inner: Arc<Chunking>,
}

#[pymethods]
impl PyFileChunks {
    /// Whether the file was analysed as ZIP (else it was cut into simple 1 MiB chunks).
    #[getter]
    fn zip_analysed(&self) -> bool {
        self.inner.zip_analysed()
    }

    /// The file's size.
    #[getter]
    fn size(&self) -> usize {
        self.inner.size()
    }

    /// The root: `(b"", size)`.
    #[getter]
    fn root<'py>(&self, py: Python<'py>) -> PyLeaf<'py> {
        leaf(py, self.inner.root())
    }

    /// The chunks, in file order.
    #[getter]
    fn chunks(&self) -> Vec<PyFileChunk> {
        self.inner
            .chunks()
            .iter()
            .cloned()
            .map(|inner| PyFileChunk { inner })
            .collect()
    }

    /// The leaves, in file order, as `(signature, size)`: what a host holds of this version,
    /// and what `delta` takes as the previous version's.
    fn leaves<'py>(&self, py: Python<'py>) -> Vec<PyLeaf<'py>> {
        self.inner.leaves().into_iter().map(|l| leaf(py, l)).collect()
    }

    /// Which chunks a host holding `previous` (the previous version's leaves, as `leaves`
    /// gives them) must be sent: a chunk whose signature and size a held leaf has is reused.
    ///
    /// # Errors
    /// `ValueError` for a signature longer than 20 bytes.
    fn delta(&self, py: Python<'_>, previous: Vec<(Vec<u8>, u64)>) -> PyResult<PyChunkDelta> {
        let previous = previous
            .into_iter()
            .map(|(signature, size)| {
                Ok(Leaf {
                    signature: Signature::new(&signature)?,
                    size,
                })
            })
            .collect::<Result<Vec<_>, ooxml_zip::ZipError>>()
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        let inner = Arc::clone(&self.inner);
        let delta = py.detach(move || inner.delta(&previous));
        Ok(PyChunkDelta { inner: delta })
    }

    fn __len__(&self) -> usize {
        self.inner.chunks().len()
    }

    fn __repr__(&self) -> String {
        format!(
            "FileChunks(size={}, chunks={}, zip_analysed={})",
            self.inner.size(),
            self.inner.chunks().len(),
            if self.inner.zip_analysed() { "True" } else { "False" }
        )
    }
}

/// One chunk: one leaf of the file's root.
#[pyclass(frozen, name = "FileChunk", module = "ooxml_core")]
pub struct PyFileChunk {
    inner: Chunk,
}

#[pymethods]
impl PyFileChunk {
    /// `entry` (a whole small entry), `header`, `data`, `final` (the central directory and
    /// end records) or `simple` (a piece of a file not analysed as ZIP).
    #[getter]
    fn kind(&self) -> &'static str {
        self.inner.kind.name()
    }

    /// The entry's name for `entry`, `header` and `data`; `""` otherwise.
    #[getter]
    fn name(&self) -> &str {
        &self.inner.name
    }

    /// Where it starts in the file.
    #[getter]
    fn offset(&self) -> usize {
        self.inner.range.start
    }

    /// Its size in bytes.
    #[getter]
    fn size(&self) -> usize {
        self.inner.size()
    }

    /// Its signature.
    #[getter]
    fn signature<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, self.inner.signature.as_bytes())
    }

    /// Whether its signature is random bytes (a final chunk over 1 MiB): never reused.
    #[getter]
    fn random(&self) -> bool {
        self.inner.random
    }

    /// Its subchunks when over 1 MiB, as `(offset, size, signature)` (8 random bytes each);
    /// empty otherwise.
    #[getter]
    fn subchunks<'py>(&self, py: Python<'py>) -> Vec<(usize, usize, Bound<'py, PyBytes>)> {
        self.inner
            .subchunks
            .iter()
            .map(|s| (s.range.start, s.range.len(), PyBytes::new(py, s.signature.as_bytes())))
            .collect()
    }

    fn __repr__(&self) -> String {
        format!(
            "FileChunk(kind={:?}, name={:?}, offset={}, size={}, signature={})",
            self.inner.kind.name(),
            self.inner.name,
            self.inner.range.start,
            self.inner.size(),
            self.inner.signature.to_hex()
        )
    }
}

/// A new version's chunks against the previous version's leaves (`FileChunks.delta`).
#[pyclass(frozen, name = "ChunkDelta", module = "ooxml_core")]
pub struct PyChunkDelta {
    inner: Delta,
}

#[pymethods]
impl PyChunkDelta {
    /// The indexes of the chunks that travel, in file order.
    #[getter(new)]
    fn new_chunks(&self) -> Vec<usize> {
        self.inner.new_chunks()
    }

    /// The indexes of the chunks the host holds already.
    #[getter]
    fn held(&self) -> Vec<usize> {
        self.inner.held_chunks()
    }

    /// `(index, earlier)` for each chunk whose leaf is an earlier new chunk's of this version
    /// (it travels once, as the earlier one).
    #[getter]
    fn repeated(&self) -> Vec<(usize, usize)> {
        self.inner
            .reuse()
            .iter()
            .enumerate()
            .filter_map(|(i, r)| match r {
                Reuse::Earlier(first) => Some((i, *first)),
                Reuse::Held | Reuse::New => None,
            })
            .collect()
    }

    /// The bytes that travel: the new chunks' sizes.
    #[getter]
    fn new_bytes(&self) -> u64 {
        self.inner.new_bytes()
    }

    fn __repr__(&self) -> String {
        format!(
            "ChunkDelta(new={}, held={}, new_bytes={})",
            self.inner.new_chunks().len(),
            self.inner.held_chunks().len(),
            self.inner.new_bytes()
        )
    }
}
