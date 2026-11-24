import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Dialog {
    id: root
    title: "Alihkan Venue: " + currentVenue + " -> " + targetVenue
    modal: true
    standardButtons: Dialog.Cancel

    property string currentVenue: "Spot"
    property string targetVenue: "UsdsFutures"
    property string engineMode: "Ready"
    property int openPositionsCount: 0
    property string selectedPolicy: "drain"

    signal confirmSwitch(string targetVenue, string policy)

    readonly property bool blocked: engineMode === "Halted" || engineMode === "Killed"

    ColumnLayout {
        spacing: 10

        Label {
            text: "Posisi terbuka saat ini: " + root.openPositionsCount
        }
        RadioButton {
            text: "Kelola sampai selesai (Draining — default)"
            checked: root.selectedPolicy === "drain"
            onCheckedChanged: if (checked) root.selectedPolicy = "drain"
        }
        RadioButton {
            text: "Wajib flat dulu"
            checked: root.selectedPolicy === "refuse"
            onCheckedChanged: if (checked) root.selectedPolicy = "refuse"
        }
        RadioButton {
            text: "Tutup semua sekarang (tahan 1,5 detik + nonce)"
            checked: root.selectedPolicy === "flatten"
            onCheckedChanged: if (checked) root.selectedPolicy = "flatten"
        }
        Label {
            visible: root.blocked
            color: "#ef4444"
            text: "Peralihan ditolak saat engine dalam mode " + root.engineMode
        }
        Button {
            text: root.selectedPolicy === "flatten" ? "Tahan 1,5s untuk Flatten & Alihkan" : "Alihkan Venue"
            enabled: !root.blocked
            onClicked: {
                root.confirmSwitch(root.targetVenue, root.selectedPolicy)
                root.close()
            }
        }
    }
}
