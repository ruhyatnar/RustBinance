import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createBotStream, SESSION_TOKEN_KEY, type WebSocketLike } from './stream.svelte';

class FakeWebSocket implements WebSocketLike {
	readonly url: string;
	readyState = 0;
	sent: string[] = [];
	onopen: (() => void) | null = null;
	onmessage: ((ev: { data: string }) => void) | null = null;
	onclose: (() => void) | null = null;
	onerror: (() => void) | null = null;

	constructor(url: string) {
		this.url = url;
	}

	send(data: string): void {
		this.sent.push(data);
	}

	close(): void {
		this.readyState = 3;
		this.onclose?.();
	}

	emitOpen(): void {
		this.readyState = 1;
		this.onopen?.();
	}

	emitJson(obj: unknown): void {
		this.onmessage?.({ data: JSON.stringify(obj) });
	}
}

const SAMPLE_SNAPSHOT = {
	venues: {
		active: 'spot',
		spot: 'Active',
		futures: 'Disabled',
		last_switch_ms: 0,
		switches_today: 0
	},
	engine: {
		mode: 'Ready',
		build_mode: 'paper',
		uptime_s: 120,
		ws_latency_ms: 12,
		clock_drift_ms: 1,
		rate: { weight_used: 50, weight_limit: 6000, orders_10s: 0 }
	},
	risk: {
		equity: '1012.40',
		day_pnl_pct: '0.82',
		week_pnl_pct: '1.9',
		drawdown_pct: '-1.1',
		heat_pct: '0.9',
		heat_max_pct: '2.0',
		open_positions: 1,
		max_positions: 4,
		btc_guard: 'SECURE',
		limits: { daily_halt_pct: '-2.0', kill_pct: '-10.0' }
	},
	positions: [],
	orders: [],
	signals_shadow: [],
	alerts: []
};

describe('createBotStream (Svelte 5 runes store)', () => {
	beforeEach(() => {
		sessionStorage.clear();
		localStorage.clear();
		vi.useFakeTimers();
	});

	afterEach(() => {
		vi.useRealTimers();
	});

	it('starts unauthenticated with zero data and cannot mutate', () => {
		const store = createBotStream();
		expect(store.authenticated).toBe(false);
		expect(store.snapshot).toBeNull();
		expect(store.canMutate).toBe(false);
		expect(localStorage.length).toBe(0);
	});

	it('connects with first-message WS auth (no query token), keeps token out of localStorage, and enforces readonly', async () => {
		const sockets: FakeWebSocket[] = [];
		const fetchMock = vi.fn(async () =>
			new Response(
				JSON.stringify({
					status: 'ok',
					engine_mode: 'Ready',
					auth_scope: 'readonly',
					ts_ms: 1790000000000
				}),
				{ status: 200, headers: { 'content-type': 'application/json' } }
			)
		);

		const store = createBotStream({
			fetchImpl: fetchMock,
			wsFactory: (url) => {
				const ws = new FakeWebSocket(url);
				sockets.push(ws);
				return ws;
			}
		});

		await store.connect('http://127.0.0.1:3001', 'ro-secret-token', true);
		expect(store.authenticated).toBe(true);
		expect(store.authScope).toBe('readonly');
		expect(sessionStorage.getItem(SESSION_TOKEN_KEY)).toBe('ro-secret-token');
		expect(localStorage.length).toBe(0);

		// Unlock attempt on readonly token MUST keep canMutate === false
		store.unlockControls();
		expect(store.canMutate).toBe(false);

		expect(sockets.length).toBe(1);
		const ws = sockets[0];
		expect(ws.url).toBe('ws://127.0.0.1:3001/ws/v1/stream');
		expect(ws.url).not.toContain('token');

		ws.emitOpen();
		expect(ws.sent.length).toBe(1);
		const authMsg = JSON.parse(ws.sent[0]);
		expect(authMsg.type).toBe('auth');
		expect(authMsg.payload.token).toBe('ro-secret-token');
	});

	it('detects seq gaps, requests snapshot, reconnects on close, and clears state on disconnect', async () => {
		const sockets: FakeWebSocket[] = [];
		const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
			const url = String(input);
			if (url.endsWith('/api/v1/health')) {
				return new Response(
					JSON.stringify({
						status: 'ok',
						engine_mode: 'Ready',
						auth_scope: 'full',
						ts_ms: 1790000000000
					}),
					{ status: 200 }
				);
			}
			if (url.endsWith('/api/v1/commands/request-confirm')) {
				return new Response(
					JSON.stringify({
						cmd_id: '0192f0a8-1234-7000-8000-000000000001',
						ok: true,
						code: 'ok',
						message: 'issued',
						confirm_nonce: 'nonce-xyz',
						expires_at_ms: 1790000010000
					}),
					{ status: 200 }
				);
			}
			return new Response(
				JSON.stringify({
					cmd_id: '0192f0a8-1234-7000-8000-000000000002',
					ok: true,
					code: 'ok',
					message: 'panic flatten executed'
				}),
				{ status: 200 }
			);
		});

		const store = createBotStream({
			fetchImpl: fetchMock,
			wsFactory: (url) => {
				const ws = new FakeWebSocket(url);
				sockets.push(ws);
				return ws;
			}
		});

		await store.connect('http://127.0.0.1:3001', 'full-secret', true);
		// Full token still starts locked (read-only default) until explicitly unlocked
		expect(store.canMutate).toBe(false);
		store.unlockControls();
		expect(store.canMutate).toBe(true);

		const ws1 = sockets[0];
		ws1.emitOpen();
		ws1.emitJson({ v: 1, type: 'snapshot', seq: 2, ts_ms: 1790000000100, payload: SAMPLE_SNAPSHOT });
		expect(store.snapshot?.risk.equity).toBe('1012.40');
		expect(store.lastSeq).toBe(2);

		// Simulate seq gap: 2 -> 5
		ws1.emitJson({
			v: 1,
			type: 'heartbeat',
			seq: 5,
			ts_ms: 1790000000500,
			payload: { engine_state: 'Ready', ws_latency_ms: 14, weight_used: 60, weight_limit: 6000 }
		});
		expect(store.gapResyncCount).toBe(1);
		const reqSnap = JSON.parse(ws1.sent[1]);
		expect(reqSnap.type).toBe('request_snapshot');

		// Execute confirmed panic_flatten via nonce flow
		const panicRes = await store.executeConfirmedAction('panic_flatten');
		expect(panicRes.ok).toBe(true);

		// Unexpected close triggers reconnect after backoff
		ws1.close();
		expect(store.connectionState).toBe('reconnecting');
		await vi.advanceTimersByTimeAsync(1500);
		expect(sockets.length).toBe(2);

		// Disconnect clears sessionStorage, snapshot, and locks controls
		store.disconnect();
		expect(store.authenticated).toBe(false);
		expect(store.snapshot).toBeNull();
		expect(store.canMutate).toBe(false);
		expect(sessionStorage.getItem(SESSION_TOKEN_KEY)).toBeNull();
	});
});
