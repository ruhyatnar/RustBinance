#pragma once

#include <QIcon>
#include <QObject>
#include <QString>
#include <QStringList>

class QAction;
class QMenu;
class QSystemTrayIcon;

#if HAVE_KF6
class KStatusNotifierItem;
#endif

namespace botui {

class CommandService;
class StateStore;

class Notifier : public QObject {
    Q_OBJECT
    Q_PROPERTY(bool hasSystemTray READ hasSystemTray CONSTANT)
    Q_PROPERTY(bool appQuitting READ appQuitting NOTIFY appQuittingChanged)

public:
    explicit Notifier(StateStore* store, CommandService* commands, QObject* parent = nullptr);
    ~Notifier() override;

    bool hasSystemTray() const { return m_hasSystemTray; }
    bool appQuitting() const { return m_appQuitting; }
    QStringList menuActionLabels() const;

    static QIcon renderModeIcon(const QString& buildMode, const QString& engineMode);

    Q_INVOKABLE void requestQuit();

signals:
    void openCockpitRequested();
    void appQuittingChanged();

private slots:
    void refreshTrayStatus();
    void postNotification(const QString& title, const QString& body, const QString& level);

private:
    void buildTrayMenu();

    StateStore* m_store = nullptr;
    CommandService* m_commands = nullptr;
    QMenu* m_menu = nullptr;
    QAction* m_openAction = nullptr;
    QAction* m_pauseAction = nullptr;
    QAction* m_resumeAction = nullptr;
    QAction* m_quitAction = nullptr;

#if HAVE_KF6
    KStatusNotifierItem* m_sni = nullptr;
#endif
    QSystemTrayIcon* m_qtTray = nullptr;

    bool m_hasSystemTray = false;
    bool m_appQuitting = false;
};

} // namespace botui
