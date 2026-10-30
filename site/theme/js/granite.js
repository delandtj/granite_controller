// Granite theme: tag reference tables by their first header so the CSS
// can draw them. Markdown tables carry no classes, so this is the only
// hook. Without JavaScript the tables still render as plain tables.
(function () {
    "use strict";

    function headerText(th) {
        return (th.textContent || "").trim().toLowerCase();
    }

    function tagTable(table) {
        var heads = table.querySelectorAll("thead th");
        if (!heads.length) {
            return;
        }
        var first = headerText(heads[0]);

        if (first === "address") {
            table.classList.add("regmap");
            return;
        }
        if (first === "ref") {
            table.classList.add("refdes");
            return;
        }
        if (first !== "pin") {
            return;
        }

        table.classList.add("pinout");
        var pinCols = [];
        var legendCols = [];
        heads.forEach(function (th, i) {
            var t = headerText(th);
            if (t === "pin") {
                pinCols.push(i);
                th.classList.add("pin-head");
            } else if (t === "legend") {
                legendCols.push(i);
            }
        });

        table.querySelectorAll("tbody tr").forEach(function (tr) {
            var cells = tr.children;
            pinCols.forEach(function (i) {
                var td = cells[i];
                if (!td) {
                    return;
                }
                var n = (td.textContent || "").trim();
                if (!/^[0-9]{1,2}$/.test(n)) {
                    return;
                }
                var pad = document.createElement("span");
                pad.className = n === "1" ? "pad pad-1" : "pad";
                pad.textContent = n;
                td.textContent = "";
                td.appendChild(pad);
                td.classList.add("pin-cell");
            });
            legendCols.forEach(function (i) {
                if (cells[i]) {
                    cells[i].classList.add("legend-cell");
                }
            });
        });
    }

    function run() {
        document.querySelectorAll(".content table").forEach(tagTable);
    }

    if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", run);
    } else {
        run();
    }
})();
