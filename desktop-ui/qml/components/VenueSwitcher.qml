import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

RowLayout {
    id: root
    spacing: 8

    property string activeVenue: "spot"
    property string spotState: "Active"
    property string futuresState: "Disabled"
    property bool readOnly: false

    signal requestSwitch(string targetVenue)

    visible: !root.readOnly

    Button {
        text: "SPOT (" + root.spotState + ")"
        highlighted: root.activeVenue === "spot"
        onClicked: root.requestSwitch("spot")
    }

    Button {
        text: "FUTURES (" + root.futuresState + ") · ISOLATED maks 3x"
        highlighted: root.activeVenue === "futures"
        onClicked: root.requestSwitch("futures")
    }
}
