// Fills the download links with the assets of the latest published release.
// Without JavaScript or when GitHub does not answer, the links stay on the releases page.
(function () {
  "use strict";

  var REPO = "Gronck2/wow-tools";

  function pick(assets, test) {
    for (var i = 0; i < assets.length; i++) {
      if (test(assets[i].name.toLowerCase())) return assets[i];
    }
    return null;
  }

  function setLink(kind, asset) {
    if (!asset) return;
    document.querySelectorAll('[data-asset="' + kind + '"]').forEach(function (a) {
      a.href = asset.browser_download_url;
    });
  }

  fetch("https://api.github.com/repos/" + REPO + "/releases/latest", { headers: { Accept: "application/vnd.github+json" } })
    .then(function (r) {
      return r.ok ? r.json() : null;
    })
    .then(function (rel) {
      if (!rel || !rel.assets) return;
      var assets = rel.assets;
      setLink(
        "setup",
        pick(assets, function (n) {
          return n.indexOf("setup") >= 0 && n.slice(-4) === ".exe";
        })
      );
      setLink(
        "portable",
        pick(assets, function (n) {
          return n.slice(-13) === "-portable.zip";
        })
      );
      setLink(
        "exe",
        pick(assets, function (n) {
          return n.indexOf("setup") < 0 && n.slice(-4) === ".exe";
        })
      );
      var version = (rel.tag_name || "").replace(/^v/, "");
      var date = rel.published_at ? new Date(rel.published_at) : null;
      document.querySelectorAll("[data-release]").forEach(function (el) {
        var text = el.getAttribute("data-release").replace("{version}", version);
        if (date) text += " · " + date.toLocaleDateString(document.documentElement.lang, { year: "numeric", month: "long", day: "numeric" });
        el.textContent = text;
        el.hidden = false;
      });
    })
    .catch(function () {});

  // Screenshots open large in a dialog; arrows, the arrow keys and a swipe go through them, Esc,
  // the close button or a click beside the picture close it. Without JavaScript the links open the PNG.
  var dialog = document.querySelector(".lightbox");
  var shots = Array.prototype.slice.call(document.querySelectorAll("a.zoom"));
  if (!dialog || !dialog.showModal || !shots.length) return;
  var big = dialog.querySelector("img");
  var caption = dialog.querySelector("figcaption");
  var count = dialog.querySelector(".lb-count");
  var current = 0;

  function show(i) {
    current = (i + shots.length) % shots.length;
    var a = shots[current];
    var img = a.querySelector("img");
    var fig = a.closest("figure");
    var cap = fig && fig.querySelector("figcaption");
    big.src = a.href;
    big.alt = img.alt;
    caption.textContent = cap ? cap.textContent : img.alt;
    count.textContent = current + 1 + " / " + shots.length;
  }

  shots.forEach(function (a, i) {
    a.addEventListener("click", function (e) {
      e.preventDefault();
      show(i);
      dialog.showModal();
    });
  });
  dialog.querySelector(".lb-prev").addEventListener("click", function () {
    show(current - 1);
  });
  dialog.querySelector(".lb-next").addEventListener("click", function () {
    show(current + 1);
  });
  dialog.querySelector(".lb-close").addEventListener("click", function () {
    dialog.close();
  });
  dialog.addEventListener("click", function (e) {
    if (e.target === dialog || e.target.tagName === "FIGURE") dialog.close();
  });
  dialog.addEventListener("keydown", function (e) {
    if (e.key === "ArrowLeft") show(current - 1);
    else if (e.key === "ArrowRight") show(current + 1);
  });
  dialog.addEventListener("close", function () {
    shots[current].focus();
  });
  var startX = null;
  dialog.addEventListener(
    "touchstart",
    function (e) {
      startX = e.touches.length === 1 ? e.touches[0].clientX : null;
    },
    { passive: true }
  );
  dialog.addEventListener("touchend", function (e) {
    if (startX === null) return;
    var dx = e.changedTouches[0].clientX - startX;
    startX = null;
    if (Math.abs(dx) > 50) show(dx < 0 ? current + 1 : current - 1);
  });
})();
