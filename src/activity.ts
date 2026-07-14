export type ProviderState = "ready" | "partial" | "available" | "planned" | "standby";

export type ProviderStatus = {
  state: ProviderState;
};

export type ActivityDuration = {
  durationSeconds?: number;
  durationMinutes: number;
};

export function eventDurationSeconds(event: ActivityDuration) {
  return event.durationSeconds ?? event.durationMinutes * 60;
}

export function totalDurationSeconds(events: ActivityDuration[]) {
  return events.reduce((sum, event) => sum + eventDurationSeconds(event), 0);
}

export function countUsableProviders(providers: ProviderStatus[]) {
  return providers.filter((provider) =>
    ["ready", "available", "partial"].includes(provider.state)
  ).length;
}

export function formatDuration(event: ActivityDuration) {
  const seconds = eventDurationSeconds(event);

  if (seconds < 60) {
    return `${Math.max(1, seconds)}s`;
  }

  return `${Math.floor(seconds / 60)}m`;
}

export function formatTotalDuration(seconds: number) {
  if (seconds < 60) {
    return `${Math.max(0, seconds)}s`;
  }

  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m`;
  }

  const hours = Math.floor(minutes / 60);
  const restMinutes = minutes % 60;
  return restMinutes ? `${hours}h ${restMinutes}m` : `${hours}h`;
}
