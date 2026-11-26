import QtQuick
import QtQuick.Layouts
import BinanceBot

PanelCard {
    id: chartCard
    title: "CANDLES · " + symbol + " (" + timeframe + ")"
    subtitle: bars ? (bars.length + " bars") : "0 bars"

    property string symbol: "BTCUSDT"
    property string timeframe: "15m"
    property var bars: []
    property var positionOverlay: ({})
    property var fmt
    property int chartHeight: 240

    onBarsChanged: candleCanvas.requestPaint()
    onPositionOverlayChanged: candleCanvas.requestPaint()

    Canvas {
        id: candleCanvas
        Layout.fillWidth: true
        Layout.preferredHeight: chartCard.chartHeight
        Layout.fillHeight: true

        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()

        onPaint: {
            var ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)

            var data = chartCard.bars || []
            if (!chartCard.fmt || data.length === 0 || width < 40 || height < 40) {
                ctx.fillStyle = Theme.textSecondary
                ctx.font = "12px sans-serif"
                ctx.fillText("No candlestick bars loaded for " + chartCard.symbol + " (" + chartCard.timeframe + ")", 16, height / 2)
                return
            }

            var minPrice = Infinity
            var maxPrice = -Infinity
            var i
            for (i = 0; i < data.length; ++i) {
                var lo = chartCard.fmt.toPlotNumber(data[i].low)
                var hi = chartCard.fmt.toPlotNumber(data[i].high)
                if (lo < minPrice) minPrice = lo
                if (hi > maxPrice) maxPrice = hi
            }

            var ov = chartCard.positionOverlay || {}
            var overlayKeys = ["entry", "stop", "tp1", "tp2"]
            for (i = 0; i < overlayKeys.length; ++i) {
                var k = overlayKeys[i]
                if (ov[k] && ov[k].length > 0) {
                    var pv = chartCard.fmt.toPlotNumber(ov[k])
                    if (pv > 0) {
                        if (pv < minPrice) minPrice = pv
                        if (pv > maxPrice) maxPrice = pv
                    }
                }
            }

            if (!(maxPrice > minPrice)) {
                maxPrice = minPrice + 1.0
            }
            var pad = (maxPrice - minPrice) * 0.06
            minPrice -= pad
            maxPrice += pad

            var leftPad = 8
            var rightAxisW = 84
            var topPad = 8
            var bottomPad = 12
            var plotW = Math.max(10, width - leftPad - rightAxisW)
            var plotH = Math.max(10, height - topPad - bottomPad)

            function yForPrice(p) {
                return topPad + plotH * (1.0 - (p - minPrice) / (maxPrice - minPrice))
            }

            // Subtle horizontal grid
            ctx.strokeStyle = Theme.border
            ctx.lineWidth = 1
            for (var g = 0; g <= 4; ++g) {
                var gy = topPad + (plotH * g) / 4.0
                ctx.beginPath()
                ctx.moveTo(leftPad, gy)
                ctx.lineTo(leftPad + plotW, gy)
                ctx.stroke()
            }

            // Draw candlesticks
            var stepX = plotW / data.length
            var bodyW = Math.max(2.0, Math.min(14.0, stepX * 0.68))

            for (i = 0; i < data.length; ++i) {
                var b = data[i]
                var o = chartCard.fmt.toPlotNumber(b.open)
                var h = chartCard.fmt.toPlotNumber(b.high)
                var l = chartCard.fmt.toPlotNumber(b.low)
                var c = chartCard.fmt.toPlotNumber(b.close)
                var up = c >= o
                var col = up ? "#10b981" : "#ef4444"

                var cx = leftPad + (i + 0.5) * stepX
                var yHigh = yForPrice(h)
                var yLow = yForPrice(l)
                var yOpen = yForPrice(o)
                var yClose = yForPrice(c)

                ctx.strokeStyle = col
                ctx.lineWidth = 1
                ctx.beginPath()
                ctx.moveTo(cx, yHigh)
                ctx.lineTo(cx, yLow)
                ctx.stroke()

                var topBody = Math.min(yOpen, yClose)
                var bodyH = Math.max(1.5, Math.abs(yClose - yOpen))
                ctx.fillStyle = col
                ctx.fillRect(cx - bodyW / 2.0, topBody, bodyW, bodyH)
            }

            // Draw position protection lines (Entry, SL, TP1, TP2) using exact string labels
            function drawOverlayLine(rawStr, tag, strokeCol) {
                if (!rawStr || rawStr.length === 0) return
                var num = chartCard.fmt.toPlotNumber(rawStr)
                if (num <= 0) return
                var py = yForPrice(num)
                ctx.save()
                ctx.strokeStyle = strokeCol
                ctx.lineWidth = 1.2
                ctx.setLineDash([5, 4])
                ctx.beginPath()
                ctx.moveTo(leftPad, py)
                ctx.lineTo(leftPad + plotW, py)
                ctx.stroke()
                ctx.restore()

                ctx.fillStyle = strokeCol
                ctx.font = "bold 10px monospace"
                ctx.fillText(tag + " " + chartCard.fmt.formatPrice(rawStr), leftPad + plotW + 4, py + 3)
            }

            drawOverlayLine(ov.entry, "ENT", "#38bdf8")
            drawOverlayLine(ov.stop, "SL", "#f87171")
            drawOverlayLine(ov.tp1, "TP1", "#34d399")
            drawOverlayLine(ov.tp2, "TP2", "#a7f3d0")

            // Last bar close label on right axis
            var lastCloseStr = data[data.length - 1].close
            var lastY = yForPrice(chartCard.fmt.toPlotNumber(lastCloseStr))
            ctx.fillStyle = Theme.textPrimary
            ctx.font = "11px monospace"
            ctx.fillText(chartCard.fmt.formatPrice(lastCloseStr), leftPad + plotW + 4, Math.max(12, Math.min(height - 6, lastY)))
        }
    }
}
