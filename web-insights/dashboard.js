"use strict";

(() => {
  const DAY = 86400000;
  const NS = "http://www.w3.org/2000/svg";
  const numberFormat = new Intl.NumberFormat("zh-CN");
  const dateTimeFormat = new Intl.DateTimeFormat("zh-CN", {
    timeZone: "Asia/Tokyo", year: "numeric", month: "2-digit", day: "2-digit",
    hour: "2-digit", minute: "2-digit", hour12: false,
  });
  const $ = (id) => document.getElementById(id);
  const state = { status: null, data: null, repository: "", range: "30", metric: "views", rowsShown: 50, request: 0, chart: null };
  const definitions = {
    views: { label: "浏览量 · PV", short: "PV", daily: true, unit: "次浏览" },
    visitors: { label: "独立访客 · 日 UV", short: "日 UV", daily: true, unique: true, unit: "人次（日 UV 之和）" },
    clones: { label: "Clone 次数", short: "Clones", daily: true, unit: "次 Clone" },
    cloners: { label: "独立 Clone 用户 · 每日", short: "独立 Cloners", daily: true, unique: true, unit: "人次（每日去重数之和）" },
    stars: { label: "Star 总数", short: "Stars", unit: "个 Star" },
    forks: { label: "Fork 总数", short: "Forks", unit: "个 Fork" },
    watchers: { label: "订阅人数", short: "订阅人数", unit: "位订阅者" },
    downloads: { label: "Release 累计下载", short: "累计下载", unit: "次附件下载" },
  };

  function element(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }
  function svgElement(tag, attributes = {}, text) {
    const node = document.createElementNS(NS, tag);
    Object.entries(attributes).forEach(([key, value]) => node.setAttribute(key, String(value)));
    if (text !== undefined) node.textContent = text;
    return node;
  }
  function numeric(value) { return typeof value === "number" && Number.isFinite(value) && value >= 0 ? value : null; }
  function formatted(value) { return numeric(value) === null ? "—" : numberFormat.format(value); }
  function utcToday() { return new Date().toISOString().slice(0, 10); }
  function dateKey(value) {
    if (typeof value !== "string") return null;
    const key = value.slice(0, 10);
    return /^\d{4}-\d{2}-\d{2}$/.test(key) && Number.isFinite(Date.parse(key + "T00:00:00Z")) ? key : null;
  }
  function timestamp(value) { const time = Date.parse(value); return Number.isFinite(time) ? time : null; }
  function timeLabel(value) { const time = timestamp(value); return time === null ? "暂无记录" : dateTimeFormat.format(new Date(time)) + " JST"; }
  function emptyRow(target, count, message, detail) {
    const row = element("tr");
    const cell = element("td", "table-empty", message);
    cell.colSpan = count;
    if (detail) cell.append(element("small", "", detail));
    row.append(cell);
    $(target).replaceChildren(row);
  }
  function setNotice(kind, title, message, action) {
    $("notice").className = "notice notice-" + kind;
    $("noticeTitle").textContent = title;
    $("noticeMessage").textContent = message;
    $("noticeAction").hidden = !action;
    $("noticeAction").textContent = action || "重试";
    $("noticeAction").onclick = action === "重新登录" ? () => window.location.reload() : () => loadStatus(true);
  }
  async function api(path) {
    const response = await fetch(new URL(path, window.location.href), { credentials: "same-origin", cache: "no-store", headers: { Accept: "application/json" } });
    if (response.status === 401 || response.status === 403) {
      const error = new Error("登录状态已失效或当前账号没有访问权限，请重新登录后查看。");
      error.status = response.status;
      throw error;
    }
    if (!response.ok) throw new Error("数据读取失败（HTTP " + response.status + "），请稍后重试。");
    if (!(response.headers.get("content-type") || "").includes("application/json")) throw new Error("未收到有效的数据响应。登录可能已过期，请刷新页面后重试。");
    return response.json();
  }
  function reportError(error) {
    const authenticationRequired = error.status === 401 || error.status === 403;
    setNotice("error", authenticationRequired ? "请重新登录" : "暂时无法读取数据", error.message || "请检查网络后重试。", authenticationRequired ? "重新登录" : "重试");
  }
  function setBusy(busy) {
    $("refreshButton").disabled = busy;
    $("repositorySelect").disabled = busy || !state.status?.repositories?.length;
    $("metricGrid").setAttribute("aria-busy", String(busy));
  }
  async function loadStatus(preserveSelection = false) {
    const request = ++state.request;
    setBusy(true);
    setNotice("loading", "正在读取采集记录", "正在获取最新数据；刷新页面不会触发新的采集。");
    try {
      const status = await api("./api/status");
      if (request !== state.request) return;
      if (!Array.isArray(status.repositories)) throw new Error("采集状态格式不正确，请检查服务配置。");
      state.status = status;
      const repositories = status.repositories.filter((item) => item && typeof item.repository === "string");
      const requested = preserveSelection ? state.repository : new URLSearchParams(window.location.search).get("repo");
      state.repository = repositories.some((item) => item.repository === requested) ? requested : repositories[0]?.repository || "";
      const options = repositories.map((item) => {
        const option = element("option", "", item.repository);
        option.value = item.repository;
        return option;
      });
      $("repositorySelect").replaceChildren(...(options.length ? options : [element("option", "", "暂无已配置仓库")]));
      $("repositorySelect").value = state.repository;
      if (!state.repository) {
        state.data = null;
        render();
        setNotice("warning", "还没有可展示的仓库", "仓库配置并完成首次采集后，这里会自动显示记录。");
        return;
      }
      await loadRepository(request);
    } catch (error) { if (request === state.request) reportError(error); }
    finally { if (request === state.request) setBusy(false); }
  }
  async function loadRepository(existingRequest) {
    const request = existingRequest ?? ++state.request;
    const repository = state.repository;
    setBusy(true);
    if (existingRequest === undefined) {
      state.data = null;
      render();
      setNotice("loading", "正在读取仓库记录", repository);
    }
    try {
      const data = await api("./api/repository?repo=" + encodeURIComponent(repository));
      if (request !== state.request) return;
      if (data.version !== 1 || data.repository !== repository || !Array.isArray(data.snapshots) || !Array.isArray(data.views) || !Array.isArray(data.clones)) throw new Error("仓库数据格式不正确，请检查采集文件。");
      state.data = data;
      state.rowsShown = 50;
      const url = new URL(window.location.href);
      url.searchParams.set("repo", repository);
      window.history.replaceState(null, "", url);
      render();
      renderFreshness();
    } catch (error) { if (request === state.request) reportError(error); }
    finally { if (request === state.request) setBusy(false); }
  }

  function snapshots() {
    return (state.data?.snapshots || []).filter((row) => dateKey(row.date)).slice().sort((a, b) => String(a.collectedAt || a.date).localeCompare(String(b.collectedAt || b.date)));
  }
  function latestSnapshots() { const rows = snapshots(); return { latest: rows.at(-1), previous: rows.at(-2) }; }
  function downloadTotal(snapshot) {
    if (!snapshot || !Array.isArray(snapshot.releases)) return null;
    let total = 0;
    for (const release of snapshot.releases) {
      if (!Array.isArray(release.assets)) return null;
      for (const asset of release.assets) { const count = numeric(asset.downloadCount); if (count === null) return null; total += count; }
    }
    return total;
  }
  function rowsByDate() {
    const rows = new Map();
    const rowFor = (key) => {
      if (!rows.has(key)) rows.set(key, { date: key, views: null, visitors: null, clones: null, cloners: null, stars: null, forks: null, watchers: null, downloads: null, collectedAt: null });
      return rows.get(key);
    };
    (state.data?.views || []).forEach((value) => { const key = dateKey(value.date); if (key) Object.assign(rowFor(key), { views: numeric(value.count), visitors: numeric(value.uniques) }); });
    (state.data?.clones || []).forEach((value) => { const key = dateKey(value.date); if (key) Object.assign(rowFor(key), { clones: numeric(value.count), cloners: numeric(value.uniques) }); });
    snapshots().forEach((snapshot) => {
      const key = dateKey(snapshot.date);
      Object.assign(rowFor(key), { stars: numeric(snapshot.stars), forks: numeric(snapshot.forks), watchers: numeric(snapshot.watchers), downloads: downloadTotal(snapshot), collectedAt: snapshot.collectedAt || null });
    });
    return rows;
  }
  function selectedRows(includeMissing = false) {
    const map = rowsByDate();
    const keys = [...map.keys()].sort();
    if (!keys.length) return [];
    const end = utcToday();
    const start = state.range === "all" ? keys[0] : new Date(Date.parse(end + "T00:00:00Z") - (Number(state.range) - 1) * DAY).toISOString().slice(0, 10);
    if (!includeMissing) return [...map.values()].filter((row) => row.date >= start && row.date <= end).sort((a, b) => a.date.localeCompare(b.date));
    const rows = [];
    for (let time = Date.parse(start + "T00:00:00Z"), last = Date.parse(end + "T00:00:00Z"); time <= last; time += DAY) {
      const key = new Date(time).toISOString().slice(0, 10);
      rows.push(map.get(key) || { date: key });
    }
    return rows;
  }
  function renderFreshness() {
    const data = state.data;
    const repositoryStatus = state.status?.repositories?.find((item) => item.repository === state.repository);
    const lastSuccess = repositoryStatus?.lastSuccessAt || data?.lastCollectedAt;
    const successTime = timestamp(lastSuccess);
    const errors = (state.status?.errors || []).filter((item) => item.repository === state.repository);
    const first = data?.firstCollectedAt || repositoryStatus?.firstCollectedAt;
    $("collectionMeta").textContent = "最近成功采集 " + timeLabel(lastSuccess) + (first ? " · 开始于 " + timeLabel(first) : "");
    if (errors.length) {
      setNotice("warning", "最近一次采集遇到问题", errors.map((item) => String(item.message || "采集失败")).join("；") + "。下方保留最近成功采集的数据。", "刷新记录");
    } else if (successTime === null) {
      setNotice("warning", "等待首次采集", "目前还没有成功采集记录。首次成功后可补入 GitHub 仍保留的近期流量数据。", "刷新记录");
    } else if (Date.now() - successTime > 36 * 3600000) {
      setNotice("warning", "采集数据已超过 36 小时未更新", "最近成功于 " + timeLabel(lastSuccess) + "。请检查定时工作流；这里展示的是已保存的历史数据。", "刷新记录");
    } else {
      setNotice("success", "采集正常，历史已保存", "最近成功采集于 " + timeLabel(lastSuccess) + "。图表随每日采集更新，流量记录可能由 GitHub 延迟修订。");
    }
  }
  function deltaNode(current, previous, label = "较上次快照") {
    const node = element("span", "delta-note", "尚无可比较快照");
    if (numeric(current) === null || numeric(previous) === null) return node;
    const difference = current - previous;
    node.className = "delta " + (difference > 0 ? "positive" : difference < 0 ? "negative" : "neutral");
    node.textContent = (difference > 0 ? "+" : difference < 0 ? "−" : "") + numberFormat.format(Math.abs(difference));
    const fragment = document.createDocumentFragment();
    fragment.append(node, element("span", "delta-note", label));
    return fragment;
  }
  function renderCards() {
    const map = rowsByDate();
    const daily = [...map.values()].filter((row) => row.views !== null || row.clones !== null).sort((a, b) => a.date.localeCompare(b.date));
    const complete = daily.filter((row) => row.date < utcToday());
    const latestDay = complete.at(-1) || daily.at(-1);
    const { latest, previous } = latestSnapshots();
    const cardDefinitions = [
      ["views", "浏览量 · PV", "↗"], ["visitors", "独立访客 · 日 UV", "◌"],
      ["clones", "Clone 次数", "↓"], ["cloners", "独立 Clone 用户 · 每日", "⌘"],
      ["stars", "Star 总数", "☆"], ["forks", "Fork 总数", "⑂"],
      ["watchers", "订阅人数 · Watch", "◉"], ["downloads", "Release 累计下载", "↓"],
    ];
    const cards = cardDefinitions.map(([key, label, mark], index) => {
      const definition = definitions[key];
      const value = definition.daily ? latestDay?.[key] : key === "downloads" ? downloadTotal(latest) : numeric(latest?.[key]);
      const card = element("article", "metric-card" + (index === 0 ? " primary" : ""));
      const heading = element("div", "metric-label", label);
      const marker = element("span", "metric-mark", mark);
      marker.setAttribute("aria-hidden", "true");
      heading.append(marker);
      const detail = element("div", "metric-detail");
      if (definition.daily) detail.textContent = latestDay ? latestDay.date + (latestDay.date === utcToday() ? " · 当天未结束" : " · 已结束 UTC 日") : "等待日流量记录";
      else detail.append(deltaNode(value, key === "downloads" ? downloadTotal(previous) : numeric(previous?.[key])));
      card.append(heading, element("p", "metric-value", formatted(value)), detail);
      return card;
    });
    $("metricGrid").replaceChildren(...cards);
    $("overviewDate").textContent = (latestDay ? "日流量：" + latestDay.date + "（UTC）" : "尚无日流量") + (latest ? " · 仓库快照：" + timeLabel(latest.collectedAt) : " · 尚无仓库快照");
    $("githubLink").hidden = !state.repository;
    if (state.repository) $("githubLink").href = "https://github.com/" + state.repository.split("/").map(encodeURIComponent).join("/");
  }
  function shortNumber(value) {
    if (value >= 1000000) return (value / 1000000).toFixed(value % 1000000 === 0 ? 0 : 1) + "M";
    if (value >= 1000) return (value / 1000).toFixed(value % 1000 === 0 ? 0 : 1) + "k";
    return String(value);
  }
  function showChartPoint(index) {
    const chart = state.chart;
    if (!chart || !chart.rows.length) return;
    index = Math.max(0, Math.min(chart.rows.length - 1, index));
    chart.index = index;
    const row = chart.rows[index];
    const value = numeric(row[state.metric]);
    chart.cursor.setAttribute("x1", chart.x(index));
    chart.cursor.setAttribute("x2", chart.x(index));
    chart.cursor.hidden = false;
    chart.cursor.style.display = "";
    chart.dot.style.display = value === null ? "none" : "";
    if (value !== null) { chart.dot.setAttribute("cx", chart.x(index)); chart.dot.setAttribute("cy", chart.y(value)); }
    const tooltip = $("chartTooltip");
    tooltip.replaceChildren(element("strong", "", row.date + " · UTC"), element("span", "", definitions[state.metric].short + "  " + formatted(value)));
    if (value === null) tooltip.append(element("small", "", "未采集，不按 0 计算"));
    else if (row.date === utcToday()) tooltip.append(element("small", "", "当天 UTC 统计日尚未结束"));
    tooltip.hidden = false;
    const width = $("chartWrap").clientWidth;
    const matrix = $("trendChart").getScreenCTM();
    const chartLeft = matrix ? chart.x(index) * matrix.a + matrix.e - $("chartWrap").getBoundingClientRect().left : chart.x(index) / 1000 * width;
    tooltip.style.left = Math.min(width - 82, Math.max(82, chartLeft)) + "px";
    tooltip.style.top = "4px";
  }
  function hideChartPoint() {
    $("chartTooltip").hidden = true;
    if (state.chart) { state.chart.cursor.style.display = "none"; state.chart.dot.style.display = "none"; }
  }
  function renderChart() {
    const rows = selectedRows(true);
    const definition = definitions[state.metric];
    const valid = rows.filter((row) => numeric(row[state.metric]) !== null);
    const svg = $("trendChart");
    svg.replaceChildren(svgElement("title", {}, definition.label + "趋势"), svgElement("desc", {}, "按 UTC 统计日展示。缺失记录显示断点，当天数据尚未结束。每日明细可在下方表格读取。"));
    $("chartEmpty").hidden = valid.length > 0;
    const sum = valid.reduce((total, row) => total + row[state.metric], 0);
    $("trendSummaryLabel").textContent = definition.daily ? definition.unique ? "所选时段日去重数之和" : "所选时段合计" : "所选时段最新快照";
    $("trendSummaryValue").textContent = valid.length ? formatted(definition.daily ? sum : valid.at(-1)[state.metric]) : "—";
    $("trendSummaryUnit").textContent = valid.length ? definition.unit : "";
    $("coverageLabel").textContent = rows.length ? valid.length + " / " + rows.length + " 天有记录" : "等待采集记录";
    $("chartFootnote").textContent = definition.unique ? "日去重人数之和不等于跨日独立人数；缺失日期显示断点，当天数据未结束。" : definition.daily ? "缺失日期显示断点；当天 UTC 数据尚未结束。" : "累计指标仅在有快照的日期显示，连线不代表两次采集之间的准确变化时点。";
    const left = 54, right = 978, top = 38, bottom = 232;
    const max = valid.length ? Math.max(...valid.map((row) => row[state.metric])) : 0;
    const rawStep = Math.max(max, 1) / 4;
    const magnitude = 10 ** Math.floor(Math.log10(rawStep));
    const step = Math.max(1, [1, 2, 2.5, 5, 10].map((part) => part * magnitude).find((part) => part >= rawStep) || magnitude * 10);
    const ceiling = Math.max(step * 4, Math.ceil(Math.max(max, 1) / step) * step);
    const x = (index) => rows.length === 1 ? (left + right) / 2 : left + index / Math.max(1, rows.length - 1) * (right - left);
    const y = (value) => bottom - value / ceiling * (bottom - top);
    for (let index = 0; index <= 4; index++) {
      const value = ceiling * index / 4;
      const position = y(value);
      svg.append(svgElement("line", { x1: left, x2: right, y1: position, y2: position, class: "chart-grid" }), svgElement("text", { x: left - 13, y: position + 3, "text-anchor": "end", class: "chart-axis-label" }, shortNumber(Number(value.toFixed(2)))));
    }
    const labels = [...new Set([0, Math.floor((rows.length - 1) / 4), Math.floor((rows.length - 1) / 2), Math.floor((rows.length - 1) * 3 / 4), rows.length - 1])].filter((index) => index >= 0);
    labels.forEach((index) => svg.append(svgElement("text", { x: x(index), y: bottom + 25, "text-anchor": index === 0 ? "start" : index === rows.length - 1 ? "end" : "middle", class: "chart-axis-label" }, rows[index].date.slice(5).replace("-", "/"))));
    let segment = [];
    const drawSegment = () => {
      if (!segment.length) return;
      const points = segment.map((point) => point.join(",")).join(" ");
      if (segment.length > 1) {
        svg.append(svgElement("polygon", { points: segment[0][0] + "," + bottom + " " + points + " " + segment.at(-1)[0] + "," + bottom, class: "chart-area" }), svgElement("polyline", { points, class: "chart-line" }));
      }
      if (valid.length <= 40 || segment.length === 1) segment.forEach(([cx, cy]) => svg.append(svgElement("circle", { cx, cy, r: segment.length === 1 ? 3.5 : 2.5, class: "chart-point" })));
      segment = [];
    };
    rows.forEach((row, index) => { const value = numeric(row[state.metric]); if (value === null) drawSegment(); else segment.push([x(index), y(value)]); });
    drawSegment();
    const cursor = svgElement("line", { y1: top, y2: bottom, class: "chart-cursor", style: "display:none" });
    const dot = svgElement("circle", { r: 5, class: "chart-focus-point", style: "display:none" });
    svg.append(cursor, dot);
    state.chart = { rows, x, y, cursor, dot, index: rows.length - 1 };
    svg.setAttribute("aria-label", definition.label + "趋势；" + valid.length + " 天有记录。使用左右方向键逐日查看。");
    hideChartPoint();
  }
  function renderRollingTables() {
    const { latest } = latestSnapshots();
    $("rollingMeta").textContent = latest ? "截至 " + timeLabel(latest.collectedAt) + " 的最近 14 天滚动榜单 · 非每日明细" : "最近 14 天滚动榜单 · 等待成功采集";
    const lists = [["referrersBody", latest?.referrers, "来源网站"], ["pathsBody", latest?.paths, "热门页面"]];
    lists.forEach(([target, list, label]) => {
      if (!Array.isArray(list) || !list.length) {
        emptyRow(target, 3, latest ? "这次采集没有" + label + "记录" : "等待首次采集", "GitHub 仅提供最近 14 天最多 10 项，缺少记录不等于没有流量。");
        return;
      }
      const rows = list.slice(0, 10).map((item, index) => {
        const row = element("tr");
        const name = element("td");
        const content = element("div", "source-name");
        content.append(element("span", "rank", String(index + 1).padStart(2, "0")));
        const text = element("div");
        if (target === "pathsBody") {
          const title = element("span", "path-title", item.title || item.path || "未命名页面");
          title.title = String(item.title || item.path || "");
          text.append(title, element("span", "path-code", item.path || "—"));
        } else text.textContent = item.referrer || "未知来源";
        content.append(text); name.append(content);
        row.append(name, element("td", "numeric", formatted(item.count)), element("td", "numeric", formatted(item.uniques)));
        return row;
      });
      $(target).replaceChildren(...rows);
    });
  }
  function renderDaily() {
    const rows = selectedRows(false).sort((a, b) => b.date.localeCompare(a.date));
    const show = rows.slice(0, state.rowsShown);
    $("exportButton").disabled = rows.length === 0;
    $("moreRowsButton").hidden = rows.length <= show.length;
    $("dailyCount").textContent = rows.length ? "显示 " + show.length + " / " + rows.length + " 天已保存记录" : "所选时段暂无记录";
    if (!rows.length) { emptyRow("dailyBody", 9, "所选时段暂无每日记录", "可以切换更长时间范围，或等待首次采集完成。"); return; }
    $("dailyBody").replaceChildren(...show.map((value) => {
      const row = element("tr");
      const date = element("td", "date-cell", value.date);
      if (value.date === utcToday()) date.append(element("span", "partial-badge", "当天未结束"));
      row.append(date);
      ["views", "visitors", "clones", "cloners", "stars", "forks", "watchers", "downloads"].forEach((key) => row.append(element("td", "numeric" + (numeric(value[key]) === null ? " missing-value" : ""), formatted(value[key]))));
      return row;
    }));
  }
  function fileSize(size) {
    if (numeric(size) === null) return "—";
    if (size < 1024) return size + " B";
    if (size < 1048576) return (size / 1024).toFixed(1) + " KB";
    if (size < 1073741824) return (size / 1048576).toFixed(1) + " MB";
    return (size / 1073741824).toFixed(1) + " GB";
  }
  function renderReleases() {
    const { latest, previous } = latestSnapshots();
    const releases = latest?.releases;
    if (!Array.isArray(releases) || !releases.length) {
      $("releaseCount").textContent = latest ? "暂无 Release 附件" : "等待记录";
      emptyRow("releasesBody", 5, latest ? "尚无可展示的 Release 附件" : "等待首次采集", "自动生成的 Source code 下载量不在 GitHub API 提供的数据中。");
      return;
    }
    const previousAssets = new Map();
    (previous?.releases || []).forEach((release) => (release.assets || []).forEach((asset) => previousAssets.set(String(asset.id), asset)));
    const rows = [];
    releases.forEach((release) => (release.assets || []).forEach((asset) => {
      const row = element("tr");
      const name = element("td");
      name.append(element("span", "release-name", asset.name || "未命名附件"));
      const title = element("div", "release-title");
      title.append(element("span", "release-tag", release.tag || "未标记版本"), document.createTextNode(release.name || ""));
      name.append(title);
      const difference = element("td", "numeric");
      const last = previousAssets.get(String(asset.id));
      if (last && numeric(asset.downloadCount) !== null && numeric(last.downloadCount) !== null) {
        const change = asset.downloadCount - last.downloadCount;
        difference.classList.add(change > 0 ? "positive" : change < 0 ? "negative" : "neutral");
        difference.textContent = (change > 0 ? "+" : change < 0 ? "−" : "") + numberFormat.format(Math.abs(change));
      } else { difference.textContent = "—"; difference.title = "没有可比较的同一附件快照"; }
      row.append(name, element("td", "", dateKey(release.publishedAt) || "—"), element("td", "numeric", fileSize(asset.size)), element("td", "numeric", formatted(asset.downloadCount)), difference);
      rows.push(row);
    }));
    $("releaseCount").textContent = releases.length + " 个版本 · " + rows.length + " 个附件";
    if (rows.length) $("releasesBody").replaceChildren(...rows);
    else emptyRow("releasesBody", 5, "已发布的版本尚无附件", "自动生成的源码压缩包不计入附件下载。");
  }
  function csvCell(value) {
    if (value === null || value === undefined) return "";
    if (typeof value === "number") return String(value);
    let text = String(value);
    if (/^[=+@\-\t\r]/.test(text)) text = "'" + text;
    return '"' + text.replaceAll('"', '""') + '"';
  }
  function exportCsv() {
    const rows = selectedRows(false);
    if (!rows.length) return;
    const headers = ["repository", "date_utc", "views", "daily_unique_visitors", "clones", "daily_unique_cloners", "stars_snapshot", "forks_snapshot", "subscribers_snapshot", "release_downloads_total_snapshot", "snapshot_collected_at", "utc_day_status"];
    const lines = [headers.join(","), ...rows.map((row) => [state.repository, row.date, row.views, row.visitors, row.clones, row.cloners, row.stars, row.forks, row.watchers, row.downloads, row.collectedAt, row.date === utcToday() ? "in_progress" : "ended"].map(csvCell).join(","))];
    const url = URL.createObjectURL(new Blob(["\uFEFF" + lines.join("\r\n")], { type: "text/csv;charset=utf-8" }));
    const link = element("a");
    link.href = url;
    link.download = state.repository.replaceAll("/", "-") + "-insights-" + rows[0].date + "-" + rows.at(-1).date + ".csv";
    document.body.append(link); link.click(); link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  function render() { renderCards(); renderChart(); renderRollingTables(); renderDaily(); renderReleases(); }

  $("repositorySelect").addEventListener("change", (event) => { state.repository = event.target.value; loadRepository(); });
  $("refreshButton").addEventListener("click", () => loadStatus(true));
  $("metricSelect").addEventListener("change", (event) => { state.metric = event.target.value; renderChart(); });
  document.querySelectorAll("[data-range]").forEach((button) => button.addEventListener("click", () => {
    state.range = button.dataset.range; state.rowsShown = 50;
    document.querySelectorAll("[data-range]").forEach((item) => item.setAttribute("aria-pressed", String(item === button)));
    renderChart(); renderDaily();
  }));
  $("moreRowsButton").addEventListener("click", () => { state.rowsShown += 50; renderDaily(); });
  $("exportButton").addEventListener("click", exportCsv);
  $("trendChart").addEventListener("pointermove", (event) => {
    if (!state.chart?.rows.length) return;
    const point = event.currentTarget.createSVGPoint();
    point.x = event.clientX; point.y = event.clientY;
    const matrix = event.currentTarget.getScreenCTM();
    if (!matrix) return;
    const value = point.matrixTransform(matrix.inverse()).x;
    const index = Math.round((value - state.chart.x(0)) / Math.max(1, state.chart.x(state.chart.rows.length - 1) - state.chart.x(0)) * (state.chart.rows.length - 1));
    showChartPoint(index);
  });
  $("trendChart").addEventListener("pointerleave", hideChartPoint);
  $("trendChart").addEventListener("focus", () => showChartPoint(state.chart?.index || 0));
  $("trendChart").addEventListener("blur", hideChartPoint);
  $("trendChart").addEventListener("keydown", (event) => {
    if (!state.chart?.rows.length || !["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
    event.preventDefault();
    showChartPoint(event.key === "Home" ? 0 : event.key === "End" ? state.chart.rows.length - 1 : state.chart.index + (event.key === "ArrowLeft" ? -1 : 1));
  });
  render();
  loadStatus();
})();
