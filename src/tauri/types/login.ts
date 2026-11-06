import type { LoginReason } from '@/generated/contract';

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
  /** The engine's own reason, else the site's diagnostic code (open: it comes from the error rules). */
  reason: LoginReason | (string & {});
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
