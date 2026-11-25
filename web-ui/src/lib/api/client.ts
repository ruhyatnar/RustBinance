/**
 * Authenticated HTTP client for `bot-ipc` (§10.1, §10.3).
 *
 * Always passes bearer tokens via `Authorization: Bearer <token>` header (never query string)
 * and normalizes both `HttpErrorResponse` and `CommandResultPayload` rejections into `ApiError`.
 */

import type {
	CommandResultPayload,
	ConfirmAction,
	HealthResponse,
	KlineBar,
	SnapshotPayload,
	TradeRecord
} from './types';
import { isHttpErrorResponse } from './types';

export class ApiError extends Error {
	readonly status: number;
	readonly code: string;

	constructor(status: number, code: string, message: string) {
		super(message);
		this.name = 'ApiError';
		this.status = status;
		this.code = code;
	}
}

export type FetchLike = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

function generateUuid(): string {
	if (typeof crypto !== 'undefined' && typeof crypto.randomUUID === 'function') {
		return crypto.randomUUID();
	}
	// Fallback RFC 4122 v4 UUID for older test environments.
	return '00000000-0000-4000-8000-000000000000'.replace(/[0]/g, () =>
		Math.floor(Math.random() * 16).toString(16)
	);
}

export class IpcHttpClient {
	readonly baseUrl: string;
	readonly #token: string;
	readonly #fetch: FetchLike;

	constructor(baseUrl: string, token: string, fetchImpl: FetchLike = fetch) {
		this.baseUrl = baseUrl.replace(/\/+$/, '');
		this.#token = token;
		this.#fetch = fetchImpl;
	}

	async #request<T>(path: string, init: RequestInit = {}): Promise<T> {
		const headers: Record<string, string> = {
			Authorization: `Bearer ${this.#token}`,
			...(init.body ? { 'Content-Type': 'application/json' } : {})
		};

		const response = await this.#fetch(`${this.baseUrl}${path}`, {
			...init,
			headers
		});

		const text = await response.text();
		let parsed: unknown = null;
		if (text.length > 0) {
			try {
				parsed = JSON.parse(text);
			} catch {
				parsed = null;
			}
		}

		if (!response.ok) {
			if (isHttpErrorResponse(parsed)) {
				throw new ApiError(response.status, parsed.error.code, parsed.error.message);
			}
			if (
				typeof parsed === 'object' &&
				parsed !== null &&
				'code' in parsed &&
				'message' in parsed &&
				typeof (parsed as Record<string, unknown>).code === 'string' &&
				typeof (parsed as Record<string, unknown>).message === 'string'
			) {
				const body = parsed as { code: string; message: string };
				throw new ApiError(response.status, body.code, body.message);
			}
			throw new ApiError(
				response.status,
				'http_error',
				text || `HTTP ${response.status} ${response.statusText}`
			);
		}

		return parsed as T;
	}

	health(): Promise<HealthResponse> {
		return this.#request<HealthResponse>('/api/v1/health');
	}

	state(): Promise<SnapshotPayload> {
		return this.#request<SnapshotPayload>('/api/v1/state');
	}

	trades(fromMs?: number, toMs?: number): Promise<{ trades: TradeRecord[] }> {
		const params = new URLSearchParams();
		if (typeof fromMs === 'number') params.set('from', String(fromMs));
		if (typeof toMs === 'number') params.set('to', String(toMs));
		const qs = params.toString();
		return this.#request<{ trades: TradeRecord[] }>(`/api/v1/trades${qs ? `?${qs}` : ''}`);
	}

	klines(symbol: string, tf = '15m', limit = 200): Promise<{ symbol: string; tf: string; limit: number; klines: KlineBar[] }> {
		const params = new URLSearchParams({ symbol, tf, limit: String(limit) });
		return this.#request(`/api/v1/klines?${params.toString()}`);
	}

	pause(cmdId = generateUuid()): Promise<CommandResultPayload> {
		return this.#request('/api/v1/commands/pause', {
			method: 'POST',
			body: JSON.stringify({ cmd_id: cmdId })
		});
	}

	resume(cmdId = generateUuid()): Promise<CommandResultPayload> {
		return this.#request('/api/v1/commands/resume', {
			method: 'POST',
			body: JSON.stringify({ cmd_id: cmdId })
		});
	}

	requestConfirm(action: ConfirmAction, cmdId = generateUuid()): Promise<CommandResultPayload> {
		return this.#request('/api/v1/commands/request-confirm', {
			method: 'POST',
			body: JSON.stringify({ cmd_id: cmdId, action })
		});
	}

	panicFlatten(confirmNonce: string, cmdId = generateUuid()): Promise<CommandResultPayload> {
		return this.#request('/api/v1/commands/panic-flatten', {
			method: 'POST',
			body: JSON.stringify({ cmd_id: cmdId, confirm_nonce: confirmNonce })
		});
	}

	resetKill(confirmNonce: string, cmdId = generateUuid()): Promise<CommandResultPayload> {
		return this.#request('/api/v1/commands/reset-kill', {
			method: 'POST',
			body: JSON.stringify({ cmd_id: cmdId, confirm_nonce: confirmNonce })
		});
	}

	closePosition(positionId: string, cmdId = generateUuid()): Promise<CommandResultPayload> {
		return this.#request('/api/v1/commands/close-position', {
			method: 'POST',
			body: JSON.stringify({ cmd_id: cmdId, position_id: positionId })
		});
	}

	setParam(key: string, value: string, cmdId = generateUuid()): Promise<CommandResultPayload> {
		return this.#request('/api/v1/commands/set-param', {
			method: 'POST',
			body: JSON.stringify({ cmd_id: cmdId, key, value })
		});
	}

	disableStrategy(id: string, cmdId = generateUuid()): Promise<CommandResultPayload> {
		return this.#request('/api/v1/commands/disable-strategy', {
			method: 'POST',
			body: JSON.stringify({ cmd_id: cmdId, id })
		});
	}

	ackAlert(id: string, cmdId = generateUuid()): Promise<CommandResultPayload> {
		return this.#request('/api/v1/commands/ack-alert', {
			method: 'POST',
			body: JSON.stringify({ cmd_id: cmdId, id })
		});
	}

	switchVenue(
		target: 'spot' | 'futures',
		onOpenPositions: 'drain' | 'refuse' | 'flatten',
		confirmNonce = '',
		cmdId = generateUuid()
	): Promise<CommandResultPayload> {
		return this.#request('/api/v1/commands/switch-venue', {
			method: 'POST',
			body: JSON.stringify({
				cmd_id: cmdId,
				target,
				on_open_positions: onOpenPositions,
				confirm_nonce: confirmNonce
			})
		});
	}
}
