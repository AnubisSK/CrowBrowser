const webview = document.getElementById('webview');
const urlBar = document.getElementById('url-bar');
const loadButton = document.getElementById('load');

loadButton.addEventListener('click', () => {
    let url = urlBar.value;
    if (!url.startsWith('http://') && !url.startsWith('https://')) {
        url = 'http://' + url; // Pridajte http:// ak nie je zadané
    }
    webview.src = url; // Načítajte URL do webview
});