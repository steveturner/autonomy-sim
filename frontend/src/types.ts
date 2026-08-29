export type EntityKind = 'drone' | 'person' | 'ground_vehicle' | 'ground_station' | 'sensor';
export type Domain = 'ground' | 'air' | 'maritime' | 'space';
export type LinkType = 'mesh' | 'cellular' | 'satcom' | 'ble';

export interface Position { lat_deg: number; lon_deg: number; alt_m: number }
export interface Kinematics { speed_mps: number; heading_deg: number; vertical_speed_mps: number }
export interface MissionState { playbook: string; active_node: string; status: 'running' | 'success' | 'failure' }
export interface EntityState {
  id: string; name: string; kind: EntityKind; domain: Domain;
  position: Position; kinematics: Kinematics; mission: MissionState;
}
export interface LinkState {
  id: string; source: string; target: string; link_type: LinkType; state: 'up' | 'down';
  source_peer_id: string; target_peer_id: string;
  quality: number; distance_m: number; latency_ms: number; packet_loss: number; capacity_bps: number;
}
export interface LinkEvent {
  link_id: string; source: string; target: string; link_type: LinkType;
  state: 'up' | 'down'; changed_at_s: number;
}
export interface TrafficState {
  link_id: string; tx_bps: number; rx_bps: number; messages_per_s: number; queue_depth: number;
  document_ops_per_s: number; pending_documents: number;
}
export interface DittoPeerState {
  peer_id: string; entity_id: string; connected_peer_ids: string[]; document_count: number;
  pending_documents: number; converged: boolean; collection_versions: Record<string, number>;
}
export interface DittoDocumentState {
  collection: string; document_id: string; author_peer_id: string; revision: number;
  updated_at_s: number; replicated_to: string[]; converged: boolean;
}
export interface DittoReplicationEvent {
  collection: string; document_id: string; revision: number; from_peer_id: string;
  to_peer_id: string; link_id: string; replicated_at_s: number;
}
export interface StateEnvelope {
  schema: 'autonomy-sim/v1'; message_type: 'state'; sequence: number; sim_time_s: number;
  payload: {
    entities: EntityState[]; links: LinkState[]; link_events: LinkEvent[]; traffic: TrafficState[];
    ditto_peers: DittoPeerState[]; ditto_documents: DittoDocumentState[];
    ditto_replication_events: DittoReplicationEvent[]; czml: unknown[];
  };
}
export interface HelloEnvelope {
  schema: 'autonomy-sim/v1'; message_type: 'hello'; sequence: 0; sim_time_s: 0;
  payload: { scenario: string; tick_hz: number; server: string };
}
