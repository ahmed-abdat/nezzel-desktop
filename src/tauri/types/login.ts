export interface LoginCandidate {
  /** Opaque browser/profile identifier. Pass back unchanged; never split it. */
  id: string;
  browser: string;
  profile: string;
  /** Known session-cookie metadata exists; this is a hint, not proof of a valid session. */
  signedIn: boolean;
  appBound?: boolean;
}

export interface LoginNeeded {
  requestId: string;
  id: string;
  groupId: string;
  host: string;
  reason: string;
  status: 'needed' | 'exhausted';
  candidates: LoginCandidate[];
  /** Profile change for the next operation; no dispatcher slot is waiting. */
  changeOnly?: boolean;
}

export interface LoginUsing {
  id: string;
  groupId: string;
  host: string;
  candidate: LoginCandidate;
  selectedOnly?: boolean;
}

export interface EngineAvailability {
  aria2Available: boolean;
}

export interface MediaTransport {
  id: string;
  groupId: string;
  path: 'aria2' | 'native';
}
