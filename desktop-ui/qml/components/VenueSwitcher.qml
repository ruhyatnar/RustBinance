import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

RowLayout {
    id: root
    spacing: 8

    property string activeVenue: "Spot"
    property string spotState: "Active"
    property string futuresState: "Disabled"
    property bool readOnly: false

    signal requestSwitch(string targetVenue)

    visible: !root.readOnly

    Button {
        text: "SPOT (" + root.spotState + ")"
        highlighted: root.activeVenue === "Spot"
        onClicked: root.requestSwitch("Spot")
    }

    Button {
        text: "FUTURES (" + root.futuresState + ") · ISOLATED maks 3x"
        highlighted: root.activeVenue === "UsdsFutures"
        onClicked: root.requestSwitch("UsdsFutures")
    }
}
