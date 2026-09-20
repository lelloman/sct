import type { ErrorResponse } from "./generated/api";

export class ApiFailure extends Error {
  constructor(
    public readonly detail: ErrorResponse,
    public readonly status: number,
  ) {
    super(detail.code.replaceAll("_", " "));
  }
}
function csrf(): string {
  for (const cookie of document.cookie.split(";")) {
    const [name, ...value] = cookie.trim().split("=");
    if (name === "__Host-sct_csrf" || name === "sct_csrf")
      return decodeURIComponent(value.join("="));
  }
  return "";
}
export async function api<T>(
  path: string,
  method = "GET",
  body?: unknown,
): Promise<T> {
  if (!path.startsWith("/api/v1/") && !path.startsWith("/auth/"))
    throw new Error("Invalid service path");
  const headers = new Headers({ Accept: "application/json" });
  if (body !== undefined) headers.set("Content-Type", "application/json");
  if (method !== "GET" && method !== "HEAD")
    headers.set("x-csrf-token", csrf());
  const response = await fetch(path, {
    method,
    headers,
    credentials: "same-origin",
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!response.ok) {
    if (response.status === 401)
      window.dispatchEvent(new Event("sct:unauthorized"));
    let detail: ErrorResponse;
    try {
      detail = (await response.json()) as ErrorResponse;
    } catch {
      throw new Error(
        `The service could not complete this request (${response.status}).`,
      );
    }
    throw new ApiFailure(detail, response.status);
  }
  return (response.status === 204 ? undefined : await response.json()) as T;
}
export async function tokenSession(token: string): Promise<void> {
  const response = await fetch("/auth/session", {
    method: "POST",
    credentials: "same-origin",
    headers: { Authorization: `Bearer ${token}` },
  });
  if (!response.ok)
    throw new Error("Sign-in failed. Check that the credential is active.");
}
export function errorMessage(error: unknown): string {
  if (error instanceof ApiFailure) {
    const messages: Record<string, string> = {
      unauthorized: "Your session has ended. Please sign in again.",
      forbidden: "Your account does not have permission for this operation.",
      revision_conflict: "This item changed. Refresh it before trying again.",
      cursor_refresh_required: "This list changed. Refresh it to continue.",
      root_busy: "Storage is busy. Try again shortly.",
      quota_exhausted:
        "There is not enough allocated capacity for this change.",
      staging_exhausted:
        "Temporary storage is full. An administrator can review capacity.",
      destination_unavailable:
        "Storage is temporarily unavailable. Try again shortly.",
      invalid_request: "Check the entered values and try again.",
      not_found: "This item is no longer available.",
    };
    return messages[error.detail.code] ?? error.message;
  }
  return error instanceof Error
    ? error.message
    : "The request could not be completed.";
}
export function segment(value: string): string {
  return encodeURIComponent(value);
}
export function bytes(value: number): string {
  if (value < 1024) return `${value} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let unit = -1;
  do {
    value /= 1024;
    unit++;
  } while (value >= 1024 && unit < units.length - 1);
  return `${value.toLocaleString(undefined, { maximumFractionDigits: 1 })} ${units[unit]}`;
}
