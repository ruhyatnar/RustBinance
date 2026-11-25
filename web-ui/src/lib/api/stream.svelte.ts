/**
 * Svelte 5 runes WebSocket + HTTP state store (§10.1–§10.2, §11.5).
 *
 * Security & reliability invariants:
 * - Token stored ONLY in memory or `sessionStorage` (NEVER `localStorage`, NEVER query string).
 * - WebSocket authenticates via first message `{ type: "auth", payload: { token } }`.
 * - Read-only by default: `readonly` token hides all mutations; `full` token starts locked until
 *   the operator explicitly clicks Unlock Controls.
 * - Detects `seq` gaps on every incoming envelope and immediately requests a fresh `snapshot`.
 * - Exponential backoff reconnect with clean teardown on `disconnect()`.
 */

import { ApiError, IpcHttpClient, type FetchLike } from './client';
import {
	PROTOCOL_VERSION,
	isWireEnvelope,
	type AuthScope,
	type ClientMessage,
	type CommandResultPayload,
	type EventPayload,
	type HeartbeatPayload,
	type HelloPayload,
	type MarketTickSummary,
	type ServerMessage,
	type SnapshotPayload,
	type WireEnvelope
} from './types';

export const SESSION_TOKEN_KEY = 'binance_bot_ipc_token';
export const SESSION_URL_KEY = 'binance_bot_ipc_url';
const MAX_RETAINED_EVENTS = 250;

export type ConnectionState = 'disconnected' | 'connecting' | 'connected' | 'reconnecting';

export interface WebSocketLike {
	readonly url: string;
	readyState: number;
	onopen: (() => void) | null;
	onmessage: ((ev: { data: string }) => void) | null;
	onclose: (() => void) | null;
	onerror: (() => void) | null;
	send(data: string): void;
	close(): void;
}

export interface StreamDeps {
	fetchImpl?: FetchLike;
	wsFactory?: (url: string) => WebSocketLike;
}

export interface LoggedEvent extends EventPayload {
	seq: number;
	ts_ms: number;
}

function toWsUrl(baseUrl: string): string {
	const trimmed = baseUrl.replace(/\/+$/, '');
	if (trimmed.startsWith('https://')) {
		return `wss://${trimmed.slice('https://'.length)}/ws/v1/stream`;
	}
	if (trimmed.startsWith('http://')) {
		return `ws://${trimmed.slice('http://'.length)}/ws/v1/stream`;
	}
	return `ws://${trimmed}/ws/v1/stream`;
}

function defaultWsFactory(url: string): WebSocketLike {
	const ws = new WebSocket(url);
	return ws as unknown as WebSocketLike;
}

export function createBotStream(deps: StreamDeps = {}) {
	const fetchImpl = deps.fetchImpl ?? ((input, init) => fetch(input, init));
	const wsFactory = deps.wsFactory ?? defaultWsFactory;

	let baseUrl = $state('http://127.0.0.1:3001');
	let token = $state('');
	let authenticated = $state(false);
	let authScope = $state<AuthScope>('readonly');
	let controlsUnlocked = $state(false);

	let connectionState = $state<ConnectionState>('disconnected');
	let reconnectAttempts = $state(0);
	let lastSeq = $state<number | null>(null);
	let gapResyncCount = $state(0);
	let lastError = $state<string | null>(null);

	let hello = $state<HelloPayload | null>(null);
	let snapshot = $state<SnapshotPayload | null>(null);
	let marketTicks = $state<MarketTickSummary[]>([]);
	let events = $state<LoggedEvent[]>([]);
	let heartbeat = $state<HeartbeatPayload | null>(null);
	let lastCommandResult = $state<CommandResultPayload | null>(null);

	let activeWs: WebSocketLike | null = null;
	let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
	let clientSeq = 1;
	let httpClient: IpcHttpClient | null = null;
	let manualDisconnect = false;

	const canMutate = $derived(authenticated && authScope === 'full' && controlsUnlocked);

	function sendClientEnvelope(ws: WebSocketLike, body: ClientMessage): void {
		const env: WireEnvelope<ClientMessage> = {
			v: PROTOCOL_VERSION,
			seq: clientSeq,
			ts_ms: Date.now(),
			...body
		};
		clientSeq += 1;
		ws.send(JSON.stringify(env));
	}

	function clearReconnectTimer(): void {
		if (reconnectTimer !== null) {
			clearTimeout(reconnectTimer);
			reconnectTimer = null;
		}
	}

	function closeSocketOnly(): void {
		if (activeWs) {
			const ws = activeWs;
			activeWs = null;
			ws.onopen = null;
			ws.onmessage = null;
			ws.onclose = null;
			ws.onerror = null;
			try {
				ws.close();
			} catch {
				// Ignore close errors during teardown.
			}
		}
	}

	function scheduleReconnect(): void {
		if (manualDisconnect || !authenticated) return;
		clearReconnectTimer();
		connectionState = 'reconnecting';
		const delayMs = Math.min(10_000, 500 * 2 ** Math.min(reconnectAttempts, 5));
		reconnectAttempts += 1;
		reconnectTimer = setTimeout(() => {
			reconnectTimer = null;
			openWebSocket();
		}, delayMs);
	}

	function handleServerEnvelope(env: WireEnvelope<ServerMessage>): void {
		if (lastSeq !== null && env.seq > lastSeq + 1) {
			gapResyncCount += 1;
			if (activeWs && activeWs.readyState === 1) {
				sendClientEnvelope(activeWs, { type: 'request_snapshot', payload: {} });
			}
		}
		lastSeq = env.seq;

		switch (env.type) {
			case 'hello':
				hello = env.payload;
				break;
			case 'snapshot':
				snapshot = env.payload;
				break;
			case 'delta':
				if (snapshot) {
					snapshot = {
						...snapshot,
						venues: env.payload.venues ?? snapshot.venues,
						engine: env.payload.engine ?? snapshot.engine,
						risk: env.payload.risk ?? snapshot.risk,
						positions: env.payload.positions ?? snapshot.positions,
						orders: env.payload.orders ?? snapshot.orders,
						alerts: env.payload.alerts ?? snapshot.alerts
					};
				}
				break;
			case 'market':
				marketTicks = env.payload.ticks;
				break;
			case 'event':
				events = [{ ...env.payload, seq: env.seq, ts_ms: env.ts_ms }, ...events].slice(
					0,
					MAX_RETAINED_EVENTS
				);
				break;
			case 'heartbeat':
				heartbeat = env.payload;
				if (snapshot) {
					snapshot = {
						...snapshot,
						engine: {
							...snapshot.engine,
							mode: env.payload.engine_state,
							ws_latency_ms: env.payload.ws_latency_ms,
							rate: {
								...snapshot.engine.rate,
								weight_used: env.payload.weight_used,
								weight_limit: env.payload.weight_limit
							}
						}
					};
				}
				break;
			case 'command_result':
				lastCommandResult = env.payload;
				break;
		}
	}

	function openWebSocket(): void {
		closeSocketOnly();
		if (!authenticated || !token) return;

		connectionState = reconnectAttempts > 0 ? 'reconnecting' : 'connecting';
		const wsUrl = toWsUrl(baseUrl);
		const ws = wsFactory(wsUrl);
		activeWs = ws;

		ws.onopen = () => {
			if (activeWs !== ws) return;
			connectionState = 'connected';
			reconnectAttempts = 0;
			lastSeq = null;
			sendClientEnvelope(ws, { type: 'auth', payload: { token } });
		};

		ws.onmessage = (ev) => {
			if (activeWs !== ws) return;
			try {
				const parsed: unknown = JSON.parse(ev.data);
				if (isWireEnvelope(parsed)) {
					handleServerEnvelope(parsed as WireEnvelope<ServerMessage>);
				}
			} catch {
				// Ignore malformed frames.
			}
		};

		ws.onerror = () => {
			if (activeWs !== ws) return;
			lastError = 'WebSocket transport error';
		};

		ws.onclose = () => {
			if (activeWs !== ws) return;
			activeWs = null;
			scheduleReconnect();
		};
	}

	async function connect(
		nextBaseUrl: string,
		nextToken: string,
		persistSession = true
	): Promise<void> {
		const cleanUrl = nextBaseUrl.trim().replace(/\/+$/, '');
		const cleanToken = nextToken.trim();
		if (!cleanToken) {
			throw new ApiError(401, 'unauthorized', 'Bearer token is required');
		}

		manualDisconnect = false;
		lastError = null;
		const client = new IpcHttpClient(cleanUrl, cleanToken, fetchImpl);
		const health = await client.health();

		baseUrl = cleanUrl;
		token = cleanToken;
		httpClient = client;
		authScope = health.auth_scope;
		authenticated = true;
		// Always default to locked view-only mode on connect (§11.5).
		controlsUnlocked = false;
		reconnectAttempts = 0;

		if (typeof sessionStorage !== 'undefined') {
			if (persistSession) {
				sessionStorage.setItem(SESSION_TOKEN_KEY, cleanToken);
				sessionStorage.setItem(SESSION_URL_KEY, cleanUrl);
			} else {
				sessionStorage.removeItem(SESSION_TOKEN_KEY);
				sessionStorage.removeItem(SESSION_URL_KEY);
			}
		}

		openWebSocket();
	}

	async function restoreFromSession(): Promise<boolean> {
		if (typeof sessionStorage === 'undefined') return false;
		const savedToken = sessionStorage.getItem(SESSION_TOKEN_KEY);
		const savedUrl = sessionStorage.getItem(SESSION_URL_KEY) ?? 'http://127.0.0.1:3001';
		if (!savedToken) return false;
		try {
			await connect(savedUrl, savedToken, true);
			return true;
		} catch {
			disconnect();
			return false;
		}
	}

	function unlockControls(): void {
		if (authenticated && authScope === 'full') {
			controlsUnlocked = true;
		}
	}

	function lockControls(): void {
		controlsUnlocked = false;
	}

	function disconnect(): void {
		manualDisconnect = true;
		clearReconnectTimer();
		closeSocketOnly();
		authenticated = false;
		token = '';
		authScope = 'readonly';
		controlsUnlocked = false;
		connectionState = 'disconnected';
		reconnectAttempts = 0;
		lastSeq = null;
		hello = null;
		snapshot = null;
		marketTicks = [];
		events = [];
		heartbeat = null;
		lastCommandResult = null;
		lastError = null;
		httpClient = null;
		if (typeof sessionStorage !== 'undefined') {
			sessionStorage.removeItem(SESSION_TOKEN_KEY);
			sessionStorage.removeItem(SESSION_URL_KEY);
		}
	}

	function requireMutateClient(): IpcHttpClient {
		if (!canMutate || !httpClient) {
			throw new ApiError(403, 'forbidden', 'Controls are locked or token is read-only');
		}
		return httpClient;
	}

	async function pause(): Promise<CommandResultPayload> {
		const res = await requireMutateClient().pause();
		lastCommandResult = res;
		return res;
	}

	async function resume(): Promise<CommandResultPayload> {
		const res = await requireMutateClient().resume();
		lastCommandResult = res;
		return res;
	}

	async function closePosition(positionId: string): Promise<CommandResultPayload> {
		const res = await requireMutateClient().closePosition(positionId);
		lastCommandResult = res;
		return res;
	}

	async function setParam(key: string, value: string): Promise<CommandResultPayload> {
		const res = await requireMutateClient().setParam(key, value);
		lastCommandResult = res;
		return res;
	}

	async function disableStrategy(id: string): Promise<CommandResultPayload> {
		const res = await requireMutateClient().disableStrategy(id);
		lastCommandResult = res;
		return res;
	}

	async function ackAlert(id: string): Promise<CommandResultPayload> {
		const res = await requireMutateClient().ackAlert(id);
		lastCommandResult = res;
		return res;
	}

	async function executeConfirmedAction(
		action: 'panic_flatten' | 'reset_kill'
	): Promise<CommandResultPayload> {
		const client = requireMutateClient();
		const req = await client.requestConfirm(action);
		if (!req.ok || !req.confirm_nonce) {
			throw new ApiError(400, req.code || 'invalid_confirm_nonce', req.message);
		}
		const res =
			action === 'panic_flatten'
				? await client.panicFlatten(req.confirm_nonce)
				: await client.resetKill(req.confirm_nonce);
		lastCommandResult = res;
		return res;
	}

	async function switchVenue(
		target: 'spot' | 'futures',
		onOpenPositions: 'drain' | 'refuse' | 'flatten'
	): Promise<CommandResultPayload> {
		const client = requireMutateClient();
		let nonce = '';
		if (onOpenPositions === 'flatten') {
			const req = await client.requestConfirm('switch_venue');
			if (!req.ok || !req.confirm_nonce) {
				throw new ApiError(400, req.code || 'invalid_confirm_nonce', req.message);
			}
			nonce = req.confirm_nonce;
		}
		const res = await client.switchVenue(target, onOpenPositions, nonce);
		lastCommandResult = res;
		return res;
	}

	function getHttpClient(): IpcHttpClient | null {
		return httpClient;
	}

	return {
		get baseUrl() {
			return baseUrl;
		},
		get authenticated() {
			return authenticated;
		},
		get authScope() {
			return authScope;
		},
		get controlsUnlocked() {
			return controlsUnlocked;
		},
		get canMutate() {
			return canMutate;
		},
		get connectionState() {
			return connectionState;
		},
		get reconnectAttempts() {
			return reconnectAttempts;
		},
		get lastSeq() {
			return lastSeq;
		},
		get gapResyncCount() {
			return gapResyncCount;
		},
		get lastError() {
			return lastError;
		},
		get hello() {
			return hello;
		},
		get snapshot() {
			return snapshot;
		},
		get marketTicks() {
			return marketTicks;
		},
		get events() {
			return events;
		},
		get heartbeat() {
			return heartbeat;
		},
		get lastCommandResult() {
			return lastCommandResult;
		},
		connect,
		restoreFromSession,
		unlockControls,
		lockControls,
		disconnect,
		pause,
		resume,
		closePosition,
		setParam,
		disableStrategy,
		ackAlert,
		executeConfirmedAction,
		switchVenue,
		getHttpClient
	};
}

export const botStream = createBotStream();
