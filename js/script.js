const webview = document.getElementById('webview');
const urlInput = document.getElementById('urlInput');
const backButton = document.getElementById('backButton');
const forwardButton = document.getElementById('forwardButton');
const reloadButton = document.getElementById('reloadButton');
const downloadButton = document.getElementById('downloadButton');
const historyButton = document.getElementById('historyButton');
const homeButton = document.getElementById('homeButton');
const tabList = document.getElementById('tabList');
const addTabButton = document.getElementById('addTabButton');
const historyPopup = document.getElementById('historyPopup');
const historyPopupClose = document.getElementById('historyPopupClose');
const downloadPopup = document.getElementById('downloadPopup');
const downloadPopupClose = document.getElementById('downloadPopupClose');

let activeTab = null;
let history = []; // Array to store history
let downloads = []; // Array to store downloads

// Funkcia na uloženie histórie do localStorage
function saveHistory() {
    localStorage.setItem('history', JSON.stringify(history));
}

// Funkcia na načítanie histórie z localStorage
function loadHistory() {
    const savedHistory = localStorage.getItem('history');
    if (savedHistory) {
        history = JSON.parse(savedHistory);
    }
}

// Funkcia na uloženie stiahnutí do localStorage
function saveDownloads() {
    localStorage.setItem('downloads', JSON.stringify(downloads));
}

// Funkcia na načítanie stiahnutí z localStorage
function loadDownloads() {
    const savedDownloads = localStorage.getItem('downloads');
    if (savedDownloads) {
        downloads = JSON.parse(savedDownloads);
    }
}

// Funkcia na vytvorenie novej záložky
function createTab(url, title, favicon) {
    const tab = document.createElement('div');
    tab.className = 'tab';
    tab.setAttribute('data-url', url);
    tab.innerHTML = `<img src="${favicon}" alt="Favicon" width="16" height="16"> ${title} <span class="close-tab">&times;</span>`;
    tabList.appendChild(tab);
    tab.addEventListener('click', () => {
        setActiveTab(tab);
        webview.src = url;
    });
    tab.querySelector('.close-tab').addEventListener('click', (e) => {
        e.stopPropagation();
        tab.remove();
        if (tab === activeTab) {
            const nextTab = tabList.lastChild;
            if (nextTab) {
                setActiveTab(nextTab);
                webview.src = nextTab.getAttribute('data-url');
            } else {
                // Ak nie sú žiadne ďalšie záložky, nastav prázdny webview
                webview.src = '';
                urlInput.value = '';
            }
        }
    });
    return tab;
}

// Nastavenie aktívnej záložky
function setActiveTab(tab) {
    if (activeTab) {
        activeTab.classList.remove('active');
    }
    activeTab = tab;
    activeTab.classList.add('active');
}

// Navigácia
backButton.addEventListener('click', () => {
    if (webview.canGoBack()) {
        webview.goBack();
    }
});

forwardButton.addEventListener('click', () => {
    if (webview.canGoForward()) {
        webview.goForward();
    }
});

reloadButton.addEventListener('click', () => {
    webview.reload();
});

homeButton.addEventListener('click', () => {
    webview.src = 'https://www.google.com';
    urlInput.value = 'https://www.google.com';
});

// Funkcia na otvorenie okna histórie
function openHistoryPopup() {
    const historyList = document.getElementById('historyList');
 historyList.innerHTML = ''; // Clear previous history
    history.forEach(item => {
        const historyItem = document.createElement('div');
        historyItem.className = 'history-item';
        historyItem.innerHTML = `<span>${item}</span> <span class="remove-button" onclick="removeHistory(this)">Odstrániť</span>`;
        historyList.appendChild(historyItem);
    });
    historyPopup.style.display = 'block';
}

// Funkcia na otvorenie okna stiahnutí
function openDownloadPopup() {
    const downloadList = document.getElementById('downloadList');
    downloadList.innerHTML = ''; // Clear previous downloads
    downloads.forEach(item => {
        const downloadItem = document.createElement('div');
        downloadItem.className = 'download-item';
        downloadItem.innerHTML = `<span>${item}</span>`;
        downloadList.appendChild(downloadItem);
    });
    downloadPopup.style.display = 'block';
}

downloadButton.addEventListener('click', openDownloadPopup);

// Zatvorenie popup okna stiahnutí
downloadPopupClose.addEventListener('click', () => {
    downloadPopup.style.display = 'none';
});

// Zatvorenie popup okna histórie
historyPopupClose.addEventListener('click', () => {
    historyPopup.style.display = 'none';
});

// Zmena URL pri zadávaní
urlInput.addEventListener('keypress', (e) => {
    if (e.key === 'Enter') {
        let input = urlInput.value.trim();
        if (/^(http|https):\/\/[^\s$.?#].[^\s]*$/.test(input)) {
            webview.src = input;
            const title = input.split('/')[2]; // Získanie názvu z URL
            const favicon = `https://www.google.com/s2/favicons?domain=${title}`; // Získanie favicon
            createTab(input, title, favicon);
            setActiveTab(createTab(input, title, favicon));
            history.push(input); // Pridanie URL do histórie
            saveHistory(); // Uloženie histórie
        } else {
            webview.src = 'https://www.google.com/search?q=' + encodeURIComponent(input);
        }
    }
});

// Pridanie novej záložky
addTabButton.addEventListener('click', () => {
    const newTabUrl = 'https://www.google.com'; // Predvolená URL pre novú záložku
    const newTabTitle = 'Google'; // Predvolený názov pre novú záložku
    const favicon = 'https://www.google.com/favicon.ico'; // Predvolený favicon
    const newTab = createTab(newTabUrl, newTabTitle, favicon);
    setActiveTab(newTab);
    webview.src = newTabUrl;
});

// Zmena veľkosti sidebaru
const resizer = document.getElementById('resizer');
let isResizing = false;

resizer.addEventListener('mousedown', () => {
    isResizing = true;
});

document.addEventListener('mousemove', (e) => {
    if (isResizing) {
        const newWidth = e.clientX;
        document.getElementById('sidebar').style.width = `${newWidth}px`;
    }
});

document.addEventListener('mouseup', () => {
    isResizing = false;
});

// Inicializácia prvej záložky
loadHistory(); // Načítanie histórie pri spustení
loadDownloads(); // Načítanie stiahnutí pri spustení
createTab('https://www.google.com', 'Google', 'https://www.google.com/favicon.ico');
setActiveTab(tabList.firstChild);

// Klávesová skratka pre otvorenie okna histórie
document.addEventListener('keydown', (e) => {
    if (e.ctrlKey && e.key === 'h') {
        openHistoryPopup();
    }
});

// Funkcia na odstránenie položky z histórie
function removeHistory(element) {
    const historyItem = element.parentElement;
    const index = Array.from(historyItem.parentElement.children).indexOf(historyItem);
    history.splice(index, 1); // Odstránenie z histórie
    saveHistory(); // Uloženie histórie
    historyItem.remove();
}

// Event pre načítanie stránky
webview.addEventListener('did-finish-load', () => {
    console.log('Page loaded successfully:', webview.getURL());
    const currentUrl = webview.getURL();
    const activeTab = document.querySelector('.tab.active');
    if (activeTab) {
        activeTab.setAttribute('data-url', currentUrl); // Uloženie aktuálnej URL do aktívnej záložky
        urlInput.value = currentUrl; // Aktualizácia URL v inpute
        webview.executeJavaScript('document.title').then((title) => {
            activeTab.innerHTML = `<img src="${activeTab.querySelector('img').src}" alt="Favicon" width="16" height="16"> ${title} <span class="close-tab">&times;</span>`; // Nastavenie textu záložky na názov stránky
        });
    }
    // Simulácia stiahnutia súboru
    const downloadFile = `Súbor z ${currentUrl}`;
    if (!downloads.includes(downloadFile)) { // Pridanie podmienky na zabránenie duplikátom
        downloads.push(downloadFile); // Pridanie do zoznamu stiahnutí
        saveDownloads(); // Uloženie stiahnutí
    }
});

// Pridanie kontextového menu
webview.addEventListener('context-menu', (event) => {
    event.preventDefault();
    const contextMenu = document.createElement('div');
    contextMenu.className = 'context-menu';
    contextMenu.style.position = 'absolute';
    contextMenu.style.left = `${event.clientX}px`;
    contextMenu.style.top = `${event.clientY}px`;
    contextMenu.innerHTML = `
        <div onclick="savePage()">Uložiť stránku</div>
        <div onclick="takeScreenshot()">Screenshot stránky</div>
        <div onclick="copyText()">Kopírovať</div>
        <div onclick="pasteText()">Vložiť</div>
    `;
    document.body.appendChild(contextMenu);

    // Zatvorenie menu pri kliknutí mimo
    document.addEventListener('click', () => {
        contextMenu.remove();
    }, { once: true });
});

// Funkcia na uloženie stránky
function savePage() {
    const currentUrl = webview.getURL();
    const link = document.createElement('a');
    link.href = currentUrl;
    link.download = 'stranka.html'; // Meno súboru
    document.body.appendChild(link);
    link.click();
    document.body.removeChild(link);
}

// Funkcia na screenshot stránky
function takeScreenshot() {
    webview.capturePage().then(image => {
        const link = document.createElement('a');
        link.href = image.toPNG();
        link.download = 'screenshot.png'; // Meno súboru
        document.body.appendChild(link);
        link.click();
        document.body.removeChild(link);
    });
}

// Funkcia na kopírovanie textu
function copyText() {
    webview.executeJavaScript('window.getSelection().toString()').then(selectedText => {
        navigator.clipboard.writeText(selectedText);
    });
}

// Funkcia na vloženie textu
function pasteText() {
    navigator.clipboard.readText().then(text => {
        webview.executeJavaScript(`document.execCommand('insertText', false, '${text}')`);
    });
};

webview.addEventListener('did-fail-load', (event) => {
    console.error('Failed to load:', event);
});