export interface EngineAvailability {
  aria2Available: boolean;
}

export interface MediaTransport {
  id: string;
  groupId: string;
  path: 'aria2' | 'native';
}
