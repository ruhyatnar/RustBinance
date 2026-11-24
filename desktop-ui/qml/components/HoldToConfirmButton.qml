import QtQuick
import BinanceBot

FocusScope {
    id: holdBtn

    property string label: "■ PANIC (Hold 1.5s)"
    property int holdMs: 1500
    property bool busy: false
    property bool enabledControl: true

    signal confirmed()

    implicitWidth: Math.max(168, btnText.implicitWidth + 26)
    implicitHeight: 32

    property real progress: 0.0
    property double pressStartMs: 0

    // Hard safety rule (§11.1 #4): single-key keyboard shortcuts MUST NOT trigger destructive actions.
    Keys.onPressed: function(event) {
        if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) {
            event.accepted = true
        }
    }

    function cancelHold() {
        holdTimer.stop()
        holdBtn.progress = 0.0
    }

    Timer {
        id: holdTimer
        interval: 25
        repeat: true
        onTriggered: {
            var elapsed = Date.now() - holdBtn.pressStartMs
            var ratio = elapsed / Math.max(1, holdBtn.holdMs)
            if (ratio >= 1.0) {
                holdTimer.stop()
                holdBtn.progress = 0.0
                holdBtn.confirmed()
            } else {
                holdBtn.progress = ratio
            }
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: 6
        color: holdBtn.enabledControl && !holdBtn.busy ? "#7f1d1d" : Theme.surfaceAltBg
        border.color: holdBtn.enabledControl && !holdBtn.busy ? "#ef4444" : Theme.border
        border.width: 1
        clip: true

        Rectangle {
            width: parent.width * holdBtn.progress
            height: parent.height
            color: "#dc2626"
            opacity: 0.75
        }

        Text {
            id: btnText
            anchors.centerIn: parent
            text: holdBtn.busy
                  ? "⏳ Confirming…"
                  : (holdBtn.progress > 0.0
                     ? (holdBtn.label + " " + Math.round(holdBtn.progress * 100) + "%")
                     : holdBtn.label)
            color: "#ffffff"
            font.pixelSize: 12
            font.bold: true
        }

        MouseArea {
            id: pointerArea
            anchors.fill: parent
            enabled: holdBtn.enabledControl && !holdBtn.busy
            acceptedButtons: Qt.LeftButton
            onPressed: {
                holdBtn.pressStartMs = Date.now()
                holdBtn.progress = 0.01
                holdTimer.start()
            }
            onReleased: holdBtn.cancelHold()
            onCanceled: holdBtn.cancelHold()
            onExited: {
                if (!containsMouse) {
                    holdBtn.cancelHold()
                }
            }
        }
    }
}
