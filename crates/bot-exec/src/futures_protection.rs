//! USDⓈ-M Futures Algo protection manager with strict make-before-break SL replacement (§4.5).

use bot_exchange::{
    AlgoOrderType, CancelAlgoOrderRequest, ExchangeGateway, GatewayError, PlaceAlgoOrderRequest,
    WorkingType,
};
use bot_types::{Price, Qty, Side, Symbol};
use rust_decimal::Decimal;

/// Closing side for a Futures position (`Long/Buy -> Sell`, `Short/Sell -> Buy`).
#[must_use]
pub const fn closing_side(position_side: Side) -> Side {
    match position_side {
        Side::Buy => Side::Sell,
        Side::Sell => Side::Buy,
    }
}

/// Active Algo protection handles for a single USDⓈ-M Futures position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlgoProtectionState {
    /// Symbol.
    pub symbol: Symbol,
    /// Direction of the open position (`Side::Buy` = Long, `Side::Sell` = Short).
    pub position_side: Side,
    /// Active Stop-Loss `algoId`.
    pub active_sl_algo_id: i64,
    /// Active Stop-Loss `clientAlgoId`.
    pub active_sl_client_id: String,
    /// Active Stop-Loss trigger price.
    pub active_sl_price: Price,
    /// Optional active Take-Profit `algoId`.
    pub active_tp_algo_id: Option<i64>,
    /// Optional active Take-Profit `clientAlgoId`.
    pub active_tp_client_id: Option<String>,
    /// Optional active Trailing-Stop `algoId`.
    pub active_trail_algo_id: Option<i64>,
}

/// Manages Futures Algo SL/TP/Trailing orders with **make-before-break** SL movement.
#[derive(Debug, Default, Clone, Copy)]
pub struct AlgoProtectionManager;

impl AlgoProtectionManager {
    /// Install initial SL (`STOP_MARKET`, `closePosition=true`) and optional TP
    /// (`TAKE_PROFIT_MARKET`, `reduceOnly=true`).
    ///
    /// # Errors
    /// Returns [`GatewayError`] if the exchange rejects the SL or TP placement.
    #[allow(clippy::too_many_arguments)]
    pub async fn install_protection(
        &self,
        gw: &dyn ExchangeGateway,
        symbol: Symbol,
        position_side: Side,
        qty: Qty,
        sl_price: Price,
        tp_price: Option<Price>,
        sl_client_id: String,
        tp_client_id: Option<String>,
    ) -> Result<AlgoProtectionState, GatewayError> {
        let close_side = closing_side(position_side);

        let sl_ack = gw
            .place_algo_order(PlaceAlgoOrderRequest {
                symbol: symbol.clone(),
                side: close_side,
                order_type: AlgoOrderType::StopMarket,
                client_algo_id: sl_client_id,
                quantity: None,
                trigger_price: Some(sl_price),
                activate_price: None,
                callback_rate: None,
                working_type: WorkingType::MarkPrice,
                close_position: true,
                reduce_only: false,
                price_protect: true,
            })
            .await?;

        let (active_tp_algo_id, active_tp_client_id) =
            if let (Some(tp_p), Some(tp_cid)) = (tp_price, tp_client_id) {
                let tp_ack = gw
                    .place_algo_order(PlaceAlgoOrderRequest {
                        symbol: symbol.clone(),
                        side: close_side,
                        order_type: AlgoOrderType::TakeProfitMarket,
                        client_algo_id: tp_cid,
                        quantity: Some(qty),
                        trigger_price: Some(tp_p),
                        activate_price: None,
                        callback_rate: None,
                        working_type: WorkingType::MarkPrice,
                        close_position: false,
                        reduce_only: true,
                        price_protect: true,
                    })
                    .await?;
                (Some(tp_ack.algo_id), Some(tp_ack.client_algo_id))
            } else {
                (None, None)
            };

        Ok(AlgoProtectionState {
            symbol,
            position_side,
            active_sl_algo_id: sl_ack.algo_id,
            active_sl_client_id: sl_ack.client_algo_id,
            active_sl_price: sl_price,
            active_tp_algo_id,
            active_tp_client_id,
            active_trail_algo_id: None,
        })
    }

    /// Move Stop-Loss using strict **make-before-break** semantics:
    /// 1. Place the new SL Algo order and wait for exchange ACK.
    /// 2. Only after the new SL is active, cancel the old SL Algo order.
    ///
    /// If step 1 fails, the old SL remains untouched and active (zero unprotected interval).
    ///
    /// # Errors
    /// Returns [`GatewayError`] if placing the new SL or canceling the old SL fails.
    pub async fn move_stop_make_before_break(
        &self,
        gw: &dyn ExchangeGateway,
        state: &mut AlgoProtectionState,
        new_sl_price: Price,
        new_sl_client_id: String,
    ) -> Result<(), GatewayError> {
        let close_side = closing_side(state.position_side);

        // Step 1 (MAKE): install new SL first. If this fails, old SL stays live!
        let new_ack = gw
            .place_algo_order(PlaceAlgoOrderRequest {
                symbol: state.symbol.clone(),
                side: close_side,
                order_type: AlgoOrderType::StopMarket,
                client_algo_id: new_sl_client_id,
                quantity: None,
                trigger_price: Some(new_sl_price),
                activate_price: None,
                callback_rate: None,
                working_type: WorkingType::MarkPrice,
                close_position: true,
                reduce_only: false,
                price_protect: true,
            })
            .await?;

        let old_id = state.active_sl_algo_id;
        let prev_client_id = state.active_sl_client_id.clone();

        // Update state immediately to the newly confirmed SL before canceling the old one.
        state.active_sl_algo_id = new_ack.algo_id;
        state.active_sl_client_id = new_ack.client_algo_id;
        state.active_sl_price = new_sl_price;

        // Step 2 (BREAK): cancel old SL now that new SL is confirmed active.
        gw.cancel_algo_order(CancelAlgoOrderRequest {
            symbol: state.symbol.clone(),
            client_algo_id: Some(prev_client_id),
            algo_id: Some(old_id),
        })
        .await?;

        Ok(())
    }

    /// Install a reduce-only `TRAILING_STOP_MARKET` Algo order.
    ///
    /// # Errors
    /// Returns [`GatewayError`] if the exchange rejects the trailing order.
    pub async fn install_trailing_stop(
        &self,
        gw: &dyn ExchangeGateway,
        state: &mut AlgoProtectionState,
        qty: Qty,
        activate_price: Price,
        callback_rate: Decimal,
        trail_client_id: String,
    ) -> Result<i64, GatewayError> {
        let close_side = closing_side(state.position_side);
        let ack = gw
            .place_algo_order(PlaceAlgoOrderRequest {
                symbol: state.symbol.clone(),
                side: close_side,
                order_type: AlgoOrderType::TrailingStopMarket,
                client_algo_id: trail_client_id,
                quantity: Some(qty),
                trigger_price: None,
                activate_price: Some(activate_price),
                callback_rate: Some(callback_rate),
                working_type: WorkingType::MarkPrice,
                close_position: false,
                reduce_only: true,
                price_protect: true,
            })
            .await?;
        state.active_trail_algo_id = Some(ack.algo_id);
        Ok(ack.algo_id)
    }
}
