use chroma_error::{ChromaError, ErrorCodes};
use chroma_index::IndexUuid;
use chroma_types::{Collection, CollectionUuid, Segment, SegmentScope, SegmentType};
use thiserror::Error;
use tonic::async_trait;
use tracing::trace;

use crate::{
    execution::operator::{Operator, OperatorType},
    sysdb::sysdb::{GetCollectionsError, GetSegmentsError, SysDb},
};

/// The `FetchSegmentOperator` fetches collection and segment information from SysDB
///
/// # Parameters
/// - `sysdb`: The SysDB reader
/// - `*_uuid`: The uuids of the collection and segments
/// - `collection_version`: The version of the collection to verify against
///
/// # Inputs
/// - No input is required
///
/// # Outputs
/// - `collection`: The collection information
/// - `*_segment`: The segment information
///
/// # Usage
/// It should be run at the start of an orchestrator to get the latest data of a collection
#[derive(Clone, Debug)]
pub struct FetchSegmentOperator {
    pub(crate) sysdb: Box<SysDb>,
    pub collection_uuid: CollectionUuid,
    pub collection_version: u32,
    // TODO: Enforce segments uuid
    pub metadata_uuid: Option<IndexUuid>,
    pub record_uuid: Option<IndexUuid>,
    pub vector_uuid: Option<IndexUuid>,
}

type FetchSegmentInput = ();

#[derive(Clone, Debug)]
pub struct FetchSegmentOutput {
    pub collection: Collection,
    pub metadata_segment: Segment,
    pub record_segment: Segment,
    pub vector_segment: Segment,
}

#[derive(Error, Debug)]
pub enum FetchSegmentError {
    #[error("Error when getting collection: {0}")]
    GetCollection(#[from] GetCollectionsError),
    #[error("Error when getting segment: {0}")]
    GetSegment(#[from] GetSegmentsError),
    #[error("No collection found")]
    NoCollection,
    #[error("No segment found")]
    NoSegment,
    // The frontend relies on ths content of the error message here to detect version mismatch
    // TODO: Refactor frontend to properly detect version mismatch
    #[error("Collection version mismatch")]
    VersionMismatch,
}

impl ChromaError for FetchSegmentError {
    fn code(&self) -> ErrorCodes {
        match self {
            FetchSegmentError::GetCollection(e) => e.code(),
            FetchSegmentError::GetSegment(e) => e.code(),
            FetchSegmentError::NoCollection => ErrorCodes::NotFound,
            FetchSegmentError::NoSegment => ErrorCodes::NotFound,
            FetchSegmentError::VersionMismatch => ErrorCodes::VersionMismatch,
        }
    }
}

impl FetchSegmentOperator {
    async fn get_collection(&self) -> Result<Collection, FetchSegmentError> {
        let collection = self
            .sysdb
            .clone()
            .get_collections(Some(self.collection_uuid), None, None, None)
            .await?
            .pop()
            .ok_or(FetchSegmentError::NoCollection)?;
        if collection.version != self.collection_version as i32 {
            Err(FetchSegmentError::VersionMismatch)
        } else {
            Ok(collection)
        }
    }
    async fn get_segment(&self, scope: SegmentScope) -> Result<Segment, FetchSegmentError> {
        let segment_type = match scope {
            SegmentScope::VECTOR => SegmentType::HnswDistributed,
            SegmentScope::METADATA => SegmentType::BlockfileMetadata,
            SegmentScope::RECORD => SegmentType::BlockfileRecord,
            SegmentScope::SQLITE => unimplemented!("Unexpected Sqlite segment"),
        };
        // TODO: Add segment uuid
        let segment_id = match scope {
            SegmentScope::VECTOR => self.vector_uuid,
            SegmentScope::METADATA => self.metadata_uuid,
            SegmentScope::RECORD => self.record_uuid,
            SegmentScope::SQLITE => unimplemented!("Unexpected Sqlite segment"),
        };
        self.sysdb
            .clone()
            .get_segments(
                segment_id.map(|id| id.0),
                Some(segment_type.into()),
                Some(scope),
                self.collection_uuid,
            )
            .await?
            // Each scope should have a single segment
            .pop()
            .ok_or(FetchSegmentError::NoSegment)
    }
}

#[async_trait]
impl Operator<FetchSegmentInput, FetchSegmentOutput> for FetchSegmentOperator {
    type Error = FetchSegmentError;

    fn get_type(&self) -> OperatorType {
        OperatorType::IO
    }

    async fn run(&self, _: &FetchSegmentInput) -> Result<FetchSegmentOutput, FetchSegmentError> {
        trace!("[{}]: {:?}", self.get_name(), self);

        Ok(FetchSegmentOutput {
            collection: self.get_collection().await?,
            metadata_segment: self.get_segment(SegmentScope::METADATA).await?,
            record_segment: self.get_segment(SegmentScope::RECORD).await?,
            vector_segment: self.get_segment(SegmentScope::VECTOR).await?,
        })
    }
}
