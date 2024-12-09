const { app, BrowserWindow } = require('electron');

function createWindow() {
    const win = new BrowserWindow({
        width: 800,
        height: 600,
        webPreferences: {
            nodeIntegration: true,
            contextIsolation: false,
            webviewTag: true,
            enableRemoteModule: true,
            allowRunningInsecureContent: true, // Povolit načítání nezabezpečeného obsahu
            webSecurity: false // Vypnout zabezpečení webu (pouze pro testování)
        },
    });

    win.setMenu(null); // Odstrániť menu úplne
    win.loadFile('index.html'); // Načítajte HTML súbor
}

app.whenReady().then(createWindow);

app.on('window-all-closed', () => {
    if (process.platform !== 'darwin') {
        app.quit();
    }
});

app.on('ready', () => {
    // Oprávnění pro načítání externích URL
    session.defaultSession.setPermissionRequestHandler((webContents, permission, callback) => {
        if (permission === 'media') {
            callback(true); // Povolit
        } else {
            callback(false); // Nepovolit
        }
    });
});

app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) {
        createWindow();
    }
});