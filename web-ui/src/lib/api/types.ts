/**
 * TypeScript mirrors of `protocol/ipc-v1.schema.json` (§10.2–§10.3).
 *
 * All monetary/decimal fields are exact strings (`DecimalString`).
 */

export const PROTOCOL_VERSION = 1 as const;

export type DecimalString = string;
export type Venue = 'spot' | 'futures';
export type VenueState = 'Disabled' | 'Activating' | 'Active' | 'Draining';
export type EngineMode =
	| 'Starting'
	| 'Syncing'
	| 'Ready'
	| 'Paused'
	| 'ReduceOnly'
	| 'Halted'
	| 'Killed';
export type TradingMode = 'paper' | 'testnet' | 'live';
export type AlertLevel = 'info' | 'warn' | 'error' | 'critical';
export type AuthScope = 'readonly' | 'full';
export type ConfirmAction = 'panic_flatten' | 'reset_kill' | 'switch_venue';
export type OnOpenPositionsArg = 'refuse' | 'drain' | 'flatten';

export interface RateStatus {
	weight_used: number;
	weight_limit: number;
	orders_10s: number;
}

export interface VenuesSummary {
	active: Venue;
	spot: VenueState;
	futures: VenueState;
	last_switch_ms: number;
	switches_today: number;
}

export interface EngineSummary {
	mode: EngineMode;
	build_mode: TradingMode;
	uptime_s: number;
	ws_latency_ms: number;
	clock_drift_ms: number;
	rate: RateStatus;
}

export interface RiskLimitsSummary {
	daily_halt_pct: DecimalString;
	kill_pct: DecimalString;
}

export interface RiskSummary {
	equity: DecimalString;
	day_pnl_pct: DecimalString;
	week_pnl_pct: DecimalString;
	drawdown_pct: DecimalString;
	heat_pct: DecimalString;
	heat_max_pct: DecimalString;
	open_positions: number;
	max_positions: number;
	btc_guard: string;
	limits: RiskLimitsSummary;
}

export interface PositionView {
	id: string;
	venue: Venue;
	symbol: string;
	strategy: string;
	state: string;
	entry: DecimalString;
	mark: DecimalString;
	qty: DecimalString;
	stop: DecimalString;
	tp1: DecimalString;
	tp2: DecimalString;
	profit_r: DecimalString;
	unrealized_quote: DecimalString;
	age_s: number;
	tp1_done: boolean;
	be_done: boolean;
}

export interface OrderView {
	client_id: string;
	venue: Venue;
	symbol: string;
	side: string;
	kind: string;
	purpose: string;
	status: string;
	price?: DecimalString | null;
	qty: DecimalString;
	filled_qty: DecimalString;
}

export interface ShadowSignalView {
	venue: Venue;
	symbol: string;
	strategy: string;
	direction: string;
	entry: DecimalString;
	stop: DecimalString;
	ts_ms: number;
}

export interface AlertView {
	id: string;
	venue?: Venue | null;
	level: AlertLevel;
	msg: string;
	acked?: boolean;
}

export interface HelloPayload {
	protocol_version: number;
	build_mode: TradingMode;
	capabilities: string[];
	engine_instance_id: string;
}

export interface SnapshotPayload {
	venues: VenuesSummary;
	engine: EngineSummary;
	risk: RiskSummary;
	positions: PositionView[];
	orders: OrderView[];
	signals_shadow: ShadowSignalView[];
	alerts: AlertView[];
}

export interface DeltaPayload {
	venues?: VenuesSummary | null;
	engine?: EngineSummary | null;
	risk?: RiskSummary | null;
	positions?: PositionView[] | null;
	orders?: OrderView[] | null;
	alerts?: AlertView[] | null;
}

export interface MarketTickSummary {
	venue: Venue;
	symbol: string;
	bid: DecimalString;
	ask: DecimalString;
	last: DecimalString;
	spread_bps: DecimalString;
	ts_ms: number;
}

export interface MarketPayload {
	ticks: MarketTickSummary[];
}

export interface EventPayload {
	venue?: Venue | null;
	symbol?: string | null;
	kind: string;
	level: AlertLevel;
	message: string;
}

export interface HeartbeatPayload {
	engine_state: EngineMode;
	ws_latency_ms: number;
	weight_used: number;
	weight_limit: number;
}

export interface CommandResultPayload {
	cmd_id: string;
	ok: boolean;
	code: string;
	message: string;
	confirm_nonce?: string | null;
	expires_at_ms?: number | null;
}

export interface KlinesPayload {
	req_id: string;
	venue: Venue;
	symbol: string;
	tf: string;
	klines: KlineBar[];
}

export interface TradesPayload {
	req_id: string;
	venue?: Venue | null;
	trades: TradeRecord[];
}

export interface GetKlinesPayload {
	req_id: string;
	venue: Venue;
	symbol: string;
	tf: string;
	limit: number;
}

export interface GetTradesPayload {
	req_id: string;
	venue?: Venue | null;
	from_ms?: number | null;
	to_ms?: number | null;
}

export type ServerMessage =
	| { type: 'hello'; payload: HelloPayload }
	| { type: 'snapshot'; payload: SnapshotPayload }
	| { type: 'delta'; payload: DeltaPayload }
	| { type: 'market'; payload: MarketPayload }
	| { type: 'event'; payload: EventPayload }
	| { type: 'heartbeat'; payload: HeartbeatPayload }
	| { type: 'command_result'; payload: CommandResultPayload }
	| { type: 'klines'; payload: KlinesPayload }
	| { type: 'trades'; payload: TradesPayload };

export type CommandBody =
	| { cmd: 'pause' }
	| { cmd: 'resume' }
	| {
			cmd: 'switch_venue';
			target: Venue;
			on_open_positions: OnOpenPositionsArg;
			confirm_nonce: string;
	  }
	| { cmd: 'set_param'; key: string; value: string }
	| { cmd: 'disable_strategy'; id: string }
	| { cmd: 'close_position'; position_id: string }
	| { cmd: 'request_confirm'; action: ConfirmAction }
	| { cmd: 'panic_flatten'; confirm_nonce: string }
	| { cmd: 'reset_kill'; confirm_nonce: string }
	| { cmd: 'ack_alert'; id: string };

export type CommandPayload = { cmd_id: string } & CommandBody;

export type ClientMessage =
	| { type: 'command'; payload: CommandPayload }
	| { type: 'auth'; payload: { token: string } }
	| { type: 'request_snapshot'; payload: Record<string, never> }
	| { type: 'get_klines'; payload: GetKlinesPayload }
	| { type: 'get_trades'; payload: GetTradesPayload };

export type WireEnvelope<T extends { type: string; payload: unknown }> = {
	v: 1;
	seq: number;
	ts_ms: number;
} & T;

export interface HttpErrorResponse {
	error: {
		code: string;
		message: string;
	};
}

export interface HealthResponse {
	status: string;
	engine_mode: EngineMode;
	auth_scope: AuthScope;
	ts_ms: number;
}

export interface TradeRecord {
	venue?: Venue;
	symbol: string;
	side?: string;
	price: DecimalString;
	qty: DecimalString;
	fee?: DecimalString;
	realized_pnl?: DecimalString;
	ts_ms: number;
}

export interface KlineBar {
	time: number;
	open: DecimalString;
	high: DecimalString;
	low: DecimalString;
	close: DecimalString;
	volume?: DecimalString;
}

const VALID_ENVELOPE_TYPES = new Set([
	'hello',
	'snapshot',
	'delta',
	'market',
	'event',
	'heartbeat',
	'command_result',
	'command',
	'auth',
	'request_snapshot',
	'klines',
	'trades',
	'get_klines',
	'get_trades'
]);

export function isWireEnvelope(value: unknown): value is WireEnvelope<ServerMessage | ClientMessage> {
	if (typeof value !== 'object' || value === null) return false;
	const obj = value as Record<string, unknown>;
	return (
		obj.v === PROTOCOL_VERSION &&
		typeof obj.type === 'string' &&
		VALID_ENVELOPE_TYPES.has(obj.type) &&
		typeof obj.seq === 'number' &&
		Number.isInteger(obj.seq) &&
		obj.seq >= 0 &&
		typeof obj.ts_ms === 'number' &&
		typeof obj.payload === 'object' &&
		obj.payload !== null
	);
}

export function isHttpErrorResponse(value: unknown): value is HttpErrorResponse {
	if (typeof value !== 'object' || value === null) return false;
	const obj = value as Record<string, unknown>;
	if (typeof obj.error !== 'object' || obj.error === null) return false;
	const err = obj.error as Record<string, unknown>;
	return typeof err.code === 'string' && typeof err.message === 'string';
}
