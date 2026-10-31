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

  document.querySelectorAll(".cmd button").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var code = btn.parentElement.querySelector("code").textContent;
      navigator.clipboard.writeText(code).then(function () {
        var label = btn.textContent;
        btn.textContent = btn.getAttribute("data-done");
        setTimeout(function () {
          btn.textContent = label;
        }, 1500);
      });
    });
  });
})();
