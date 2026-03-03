(function () {
    var themes = ["navy", "coal", "ayu", "rust", "light"];
    var scrollbox = document.querySelector(".sidebar-scrollbox");
    if (!scrollbox) return;

    var root = typeof path_to_root === "string" ? path_to_root : "";

    var link = document.createElement("a");
    link.href = root + "index.html";
    link.className = "sidebar-logo";

    themes.forEach(function (t) {
        var img = document.createElement("img");
        img.src = root + "images/logo-" + t + ".svg";
        img.alt = "tty-web";
        img.dataset.theme = t;
        link.appendChild(img);
    });

    scrollbox.insertBefore(link, scrollbox.firstChild);
})();
