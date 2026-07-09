use std::sync::Arc;
use tonic::{Request, Response, Status};

use crate::peer_client::proto::consensus_service_server::ConsensusService;
use crate::peer_client::proto::{
    AppendEntriesRequest, AppendEntriesResponse, RequestVoteRequest, RequestVoteResponse,
    InstallSnapshotRequest, InstallSnapshotResponse,
};
use crate::ConsensusEngine;

/// gRPC Server implementation for ConsensusService
pub struct ConsensusServerImpl {
    engine: Arc<ConsensusEngine>,
}

impl ConsensusServerImpl {
    pub fn new(engine: Arc<ConsensusEngine>) -> Self {
        Self { engine }
    }
}

#[tonic::async_trait]
impl ConsensusService for ConsensusServerImpl {
    async fn request_vote(
        &self,
        request: Request<RequestVoteRequest>,
    ) -> Result<Response<RequestVoteResponse>, Status> {
        let req = request.into_inner();
        
        let granted = self.engine.handle_request_vote(
            &req.candidate_id,
            req.term,
            req.last_log_index,
        );

        Ok(Response::new(RequestVoteResponse {
            term: self.engine.current_term(),
            vote_granted: granted,
        }))
    }

    async fn append_entries(
        &self,
        request: Request<AppendEntriesRequest>,
    ) -> Result<Response<AppendEntriesResponse>, Status> {
        let req = request.into_inner();
        
        // Convert proto entries to domain strings
        let entries = req.entries.into_iter().map(|e| e.data).collect();

        let success = self.engine.handle_append_entries(
            &req.leader_id,
            req.term,
            entries,
        );

        Ok(Response::new(AppendEntriesResponse {
            term: self.engine.current_term(),
            success,
            match_index: self.engine.get_last_index(),
        }))
    }

    async fn install_snapshot(
        &self,
        request: Request<InstallSnapshotRequest>,
    ) -> Result<Response<InstallSnapshotResponse>, Status> {
        let req = request.into_inner();

        self.engine.handle_install_snapshot(
            &req.leader_id,
            req.term,
            req.last_included_index,
            req.last_included_term,
            req.data,
        );

        Ok(Response::new(InstallSnapshotResponse {
            term: self.engine.current_term(),
        }))
    }
}
