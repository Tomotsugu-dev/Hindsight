import { installerPlatform, installerTotals, summarizeInstallerReleases, installerDailyRows } from "./downloads.mjs";

(() => {
  const DAY = 86400000;
  const NS = "http://www.w3.org/2000/svg";
  const numberFormat = new Intl.NumberFormat("zh-CN");
  const dateTimeFormat = new Intl.DateTimeFormat("zh-CN", {
    timeZone: "Asia/Tokyo", year: "numeric", month: "2-digit", day: "2-digit",
    hour: "2-digit", minute: "2-digit", hour12: false,
  });
  const $ = (id) => document.getElementById(id);
  const state = { status: null, data: null, repository: "", range: "30", metric: "views", installerMode: "cumulative", installerPlatform: "total", rowsShown: 50, request: 0, chart: null, installerChart: null };
  const platforms = { total: "全部平台", windows: "Windows", macos: "macOS", linux: "Linux", other: "其他平台" };
  const definitions = {
    views: { label: "浏览量 · PV", short: "PV", daily: true, unit: "次浏览" },
    visitors: { label: "独立访客 · 日 UV", short: "日 UV", daily: true, unique: true, unit: "人次（日 UV 之和）" },
    clones: { label: "Clone 次数", short: "Clones", daily: true, unit: "次 Clone" },
    cloners: { label: "独立 Clone 用户 · 每日", short: "独立 Cloners", daily: true, unique: true, unit: "人次（每日去重数之和）" },
    stars: { label: "Star 总数", short: "Stars", unit: "个 Star" },
    forks: { label: "Fork 总数", short: "Forks", unit: "个 Fork" },
    watchers: { label: "订阅人数", short: "订阅人数", unit: "位订阅者" },
    downloads: { label: "安装包累计下载", short: "安装包下载", unit: "次安装包下载" },
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
    return snapshot ? numeric(installerTotals(snapshot)?.total) : null;
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
    return rowsInRange(rowsByDate(), includeMissing);
  }
  function rowsInRange(map, includeMissing = false) {
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
      ["watchers", "订阅人数 · Watch", "◉"], ["downloads", "安装包累计下载", "↓"],
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
  function showChartPoint(index, chartKey = "chart") {
    const chart = state[chartKey];
    if (!chart || !chart.rows.length) return;
    index = Math.max(0, Math.min(chart.rows.length - 1, index));
    chart.index = index;
    const row = chart.rows[index];
    const value = numeric(row[chart.valueKey]);
    chart.cursor.setAttribute("x1", chart.x(index));
    chart.cursor.setAttribute("x2", chart.x(index));
    chart.cursor.setAttribute("visibility", "visible");
    chart.dot.setAttribute("visibility", value === null ? "hidden" : "visible");
    if (value !== null) { chart.dot.setAttribute("cx", chart.x(index)); chart.dot.setAttribute("cy", chart.y(value)); }
    const tooltip = $(chart.tooltipId);
    tooltip.replaceChildren(element("strong", "", row.date + " · UTC"), element("span", "", chart.label + "  " + formatted(value)));
    if (value === null) tooltip.append(element("small", "", chart.missingMessage || "未采集，不按 0 计算"));
    else if (row.date === utcToday()) tooltip.append(element("small", "", "当天 UTC 统计日尚未结束"));
    tooltip.hidden = false;
    const width = $(chart.wrapId).clientWidth;
    const matrix = $(chart.svgId).getScreenCTM();
    const chartLeft = matrix ? chart.x(index) * matrix.a + matrix.e - $(chart.wrapId).getBoundingClientRect().left : chart.x(index) / chart.width * width;
    tooltip.style.left = Math.min(width - 82, Math.max(82, chartLeft)) + "px";
    tooltip.style.top = "4px";
  }
  function hideChartPoint(chartKey = "chart") {
    const chart = state[chartKey];
    if (!chart) return;
    $(chart.tooltipId).hidden = true;
    chart.cursor.setAttribute("visibility", "hidden");
    chart.dot.setAttribute("visibility", "hidden");
  }
  function renderChart() {
    const rows = selectedRows(true);
    const definition = definitions[state.metric];
    const valid = rows.filter((row) => numeric(row[state.metric]) !== null);
    const sum = valid.reduce((total, row) => total + row[state.metric], 0);
    $("trendSummaryLabel").textContent = definition.daily ? definition.unique ? "所选时段日去重数之和" : "所选时段合计" : "所选时段最新快照";
    $("trendSummaryValue").textContent = valid.length ? formatted(definition.daily ? sum : valid.at(-1)[state.metric]) : "—";
    $("trendSummaryUnit").textContent = valid.length ? definition.unit : "";
    $("coverageLabel").textContent = rows.length ? valid.length + " / " + rows.length + " 天有记录" : "等待采集记录";
    $("chartFootnote").textContent = definition.unique ? "日去重人数之和不等于跨日独立人数；缺失日期显示断点，当天数据未结束。" : definition.daily ? "缺失日期显示断点；当天 UTC 数据尚未结束。" : "累计指标仅在有快照的日期显示，连线不代表两次采集之间的准确变化时点。";
    drawChart({ chartKey: "chart", svgId: "trendChart", wrapId: "chartWrap", tooltipId: "chartTooltip", emptyId: "chartEmpty", rows, valueKey: state.metric, label: definition.short });
  }
  function drawChart({ chartKey, svgId, wrapId, tooltipId, emptyId, rows, valueKey, label, missingMessage }) {
    const valid = rows.filter((row) => numeric(row[valueKey]) !== null);
    const svg = $(svgId);
    svg.replaceChildren(svgElement("title", {}, label + "趋势"), svgElement("desc", {}, "按 UTC 日期展示，缺失记录显示断点。可聚焦图表并使用左右方向键逐日查看。"));
    $(emptyId).hidden = valid.length > 0;
    // Match the SVG coordinate system to its actual CSS size. A fixed 1000px
    // viewBox shrank 14px labels to 6–7px in the half-width download panel.
    const wrap = $(wrapId);
    const width = Math.max(240, Math.round(wrap.clientWidth || 1000));
    const height = Math.max(240, Math.round(wrap.clientHeight || 288));
    svg.setAttribute("viewBox", `0 0 ${width} ${height}`);
    const left = width < 420 ? 58 : 64, right = width - 20, top = 34, bottom = height - 38;
    const max = valid.length ? Math.max(...valid.map((row) => row[valueKey])) : 0;
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
    const labelPositions = width < 500 ? [0, .5, 1] : [0, .25, .5, .75, 1];
    const labels = [...new Set(labelPositions.map((position) => Math.round((rows.length - 1) * position)))].filter((index) => index >= 0 && index < rows.length);
    labels.forEach((index) => svg.append(svgElement("text", { x: x(index), y: bottom + 27, "text-anchor": rows.length === 1 ? "middle" : index === 0 ? "start" : index === rows.length - 1 ? "end" : "middle", class: "chart-axis-label" }, rows[index].date.slice(5).replace("-", "/"))));
    let segment = [];
    const drawSegment = () => {
      if (!segment.length) return;
      const points = segment.map((point) => point.join(",")).join(" ");
      if (segment.length > 1) {
        svg.append(svgElement("polygon", { points: segment[0][0] + "," + bottom + " " + points + " " + segment.at(-1)[0] + "," + bottom, class: "chart-area" }), svgElement("polyline", { points, class: "chart-line" }));
      }
      if (valid.length <= 40 || segment.length === 1) segment.forEach(([cx, cy]) => svg.append(svgElement("circle", { cx, cy, r: segment.length === 1 ? 5 : 2.5, class: "chart-point" })));
      segment = [];
    };
    rows.forEach((row, index) => { const value = numeric(row[valueKey]); if (value === null) drawSegment(); else segment.push([x(index), y(value)]); });
    drawSegment();
    if (valid.length === 1) {
      const index = rows.findIndex((row) => numeric(row[valueKey]) !== null);
      svg.append(svgElement("text", { x: x(index), y: y(valid[0][valueKey]) - 13, "text-anchor": x(index) > right - 50 ? "end" : x(index) < left + 50 ? "start" : "middle", class: "chart-single-value" }, formatted(valid[0][valueKey])));
    }
    const cursor = svgElement("line", { y1: top, y2: bottom, class: "chart-cursor", visibility: "hidden" });
    const dot = svgElement("circle", { r: 5, class: "chart-focus-point", visibility: "hidden" });
    svg.append(cursor, dot);
    state[chartKey] = { rows, x, y, width, height, cursor, dot, valueKey, label, svgId, wrapId, tooltipId, missingMessage, index: Math.max(0, rows.findLastIndex((row) => numeric(row[valueKey]) !== null)) };
    svg.setAttribute("aria-label", label + "趋势；" + valid.length + " 天有记录。使用左右方向键逐日查看。");
    hideChartPoint(chartKey);
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
  function availablePlatforms() {
    const present = new Set(["total", "windows", "macos"]);
    snapshots().forEach((snapshot) => {
      const totals = installerTotals(snapshot);
      ["linux", "other"].forEach((key) => { if (numeric(totals?.[key]) > 0) present.add(key); });
      (snapshot.releases || []).forEach((release) => (release.assets || []).forEach((asset) => {
        const platform = installerPlatform(asset.name);
        if (platform) present.add(platform);
      }));
    });
    return Object.keys(platforms).filter((key) => present.has(key));
  }
  function renderInstallerDownloads() {
    const { latest } = latestSnapshots();
    const totals = latest ? installerTotals(latest) : null;
    const available = availablePlatforms();
    $("installerTotals").replaceChildren(...available.map((platform) => {
      const card = element("article", "installer-total-card platform-" + platform);
      const label = element("p", "installer-total-label", platform === "total" ? "安装包下载总计" : platforms[platform]);
      const value = numeric(totals?.[platform]);
      const detail = platform === "total" ? "所有版本累计" : value !== null && numeric(totals?.total) > 0 ? "占全部安装包 " + (value / totals.total * 100).toFixed(1) + "%" : "累计下载次数";
      card.append(label, element("strong", "installer-total-value", formatted(value)), element("p", "installer-total-detail", detail));
      return card;
    }));
    $("installerMeta").textContent = latest ? "截至 " + timeLabel(latest.collectedAt) + " · 已保存 " + new Set(snapshots().map((row) => dateKey(row.date))).size + " 个 UTC 日快照" : "首次采集后显示历史累计；从相邻两日快照开始计算每日新增。";
    const selector = $("installerPlatformSelect");
    if (!available.includes(state.installerPlatform)) state.installerPlatform = "total";
    selector.replaceChildren(...available.map((key) => {
      const option = element("option", "", platforms[key]);
      option.value = key;
      return option;
    }));
    selector.value = state.installerPlatform;
    const legend = available.filter((key) => key !== "total").map((key) => element("span", "platform-legend-item platform-" + key, platforms[key]));
    $("installerLegend").replaceChildren(...legend);
    renderInstallerVersions(latest);
    renderInstallerHistory();
  }
  function renderInstallerVersions(latest) {
    const releases = latest ? summarizeInstallerReleases(latest).slice().sort((a, b) => (timestamp(b.publishedAt) || 0) - (timestamp(a.publishedAt) || 0) || (Number(b.id) || 0) - (Number(a.id) || 0)) : [];
    const shown = releases.slice(0, 10);
    $("installerVersionCount").textContent = releases.length ? "展示 " + shown.length + " / " + releases.length + " 个版本" : "暂无版本";
    if (!shown.length) {
      $("installerVersionBars").replaceChildren(element("p", "compact-empty", latest ? "这次快照没有可用的安装包版本记录" : "等待首次采集"));
      return;
    }
    const maximum = Math.max(1, ...shown.map((release) => numeric(release.total) || 0));
    $("installerVersionBars").replaceChildren(...shown.map((release) => {
      const row = element("div", "installer-version-row");
      const heading = element("div", "installer-version-heading");
      const name = element("span", "installer-version-name", release.tag || "未标记版本");
      name.title = "发布于 " + (dateKey(release.publishedAt) || "未知日期");
      heading.append(name, element("span", "installer-version-total", formatted(release.total)));
      const chart = svgElement("svg", { viewBox: "0 0 1000 10", preserveAspectRatio: "none", class: "installer-version-bar", role: "img" });
      const description = (release.tag || "未标记版本") + "：" + ["windows", "macos", "linux", "other"].map((key) => platforms[key] + " " + formatted(release[key])).join("，");
      chart.setAttribute("aria-label", description);
      chart.append(svgElement("title", {}, description), svgElement("rect", { x: 0, y: 0, width: 1000, height: 10, rx: 3, class: "installer-bar-track" }));
      let x = 0;
      ["windows", "macos", "linux", "other"].forEach((key) => {
        const value = numeric(release[key]);
        if (value === null || value === 0) return;
        const width = value / maximum * 1000;
        const segment = svgElement("rect", { x, y: 0, width, height: 10, rx: 2, class: "installer-bar-segment platform-" + key });
        segment.append(svgElement("title", {}, platforms[key] + "：" + formatted(value)));
        chart.append(segment);
        x += width;
      });
      row.append(heading, chart);
      return row;
    }));
  }
  function renderInstallerHistory() {
    const key = state.installerMode;
    const dailyRows = installerDailyRows(snapshots(), state.installerPlatform);
    const rows = rowsInRange(new Map(dailyRows.map((row) => [row.date, row])), true);
    const valid = rows.filter((row) => numeric(row[key]) !== null);
    const latest = valid.at(-1);
    const sum = valid.reduce((total, row) => total + row[key], 0);
    const isIncrease = key === "increase";
    $("installerRangeLabel").textContent = "跟随上方时间范围 · " + (state.range === "all" ? "全部记录" : state.range + " 天") + " · UTC";
    $("installerSummaryLabel").textContent = isIncrease ? "已知日期新增之和" : "所选时段最新累计";
    $("installerSummaryValue").textContent = valid.length ? formatted(isIncrease ? sum : latest[key]) : "—";
    $("installerCoverage").textContent = rows.length ? valid.length + " / " + rows.length + " 天" + (isIncrease ? "可比较" : "有快照") + (valid.length === 1 && !isIncrease ? dailyRows.length === 1 ? " · 已记录首个累计基线" : " · 当前范围仅一次快照" : "") : "等待下载快照";
    $("installerChartEmptyTitle").textContent = isIncrease ? "还不能计算每日新增" : "这个时段没有安装包下载快照";
    $("installerChartEmptyMessage").textContent = isIncrease ? "需要相邻 UTC 日期的快照。首日和断采后的第一天保留为未知，不按 0 计算。" : "切换更长时间范围，或等待首次采集完成。";
    $("installerFootnote").textContent = isIncrease ? "新增量是相邻 UTC 日期快照之间的增量，覆盖两次采集的间隔；首日、断采及无法比较的变化保留为空。" : "首个快照是此前历史下载的累计基线，不是首日新增；断采日期显示断点。下载不代表实际安装。";
    $("installerChart").dataset.platform = state.installerPlatform;
    drawChart({ chartKey: "installerChart", svgId: "installerChart", wrapId: "installerChartWrap", tooltipId: "installerChartTooltip", emptyId: "installerChartEmpty", rows, valueKey: key, label: platforms[state.installerPlatform] + (isIncrease ? "新增下载" : "累计下载"), missingMessage: isIncrease ? "没有相邻 UTC 快照，新增量未知" : "未保存快照，不按 0 计算" });
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
    const headers = ["repository", "date_utc", "views", "daily_unique_visitors", "clones", "daily_unique_cloners", "stars_snapshot", "forks_snapshot", "subscribers_snapshot", "installer_downloads_total_snapshot", "snapshot_collected_at", "utc_day_status"];
    const lines = [headers.join(","), ...rows.map((row) => [state.repository, row.date, row.views, row.visitors, row.clones, row.cloners, row.stars, row.forks, row.watchers, row.downloads, row.collectedAt, row.date === utcToday() ? "in_progress" : "ended"].map(csvCell).join(","))];
    const url = URL.createObjectURL(new Blob(["\uFEFF" + lines.join("\r\n")], { type: "text/csv;charset=utf-8" }));
    const link = element("a");
    link.href = url;
    link.download = state.repository.replaceAll("/", "-") + "-insights-" + rows[0].date + "-" + rows.at(-1).date + ".csv";
    document.body.append(link); link.click(); link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  function render() { renderCards(); renderChart(); renderInstallerDownloads(); renderRollingTables(); renderDaily(); }

  $("repositorySelect").addEventListener("change", (event) => { state.repository = event.target.value; loadRepository(); });
  $("refreshButton").addEventListener("click", () => loadStatus(true));
  $("metricSelect").addEventListener("change", (event) => { state.metric = event.target.value; renderChart(); });
  document.querySelectorAll("[data-range]").forEach((button) => button.addEventListener("click", () => {
    state.range = button.dataset.range; state.rowsShown = 50;
    document.querySelectorAll("[data-range]").forEach((item) => item.setAttribute("aria-pressed", String(item === button)));
    renderChart(); renderInstallerHistory(); renderDaily();
  }));
  $("installerPlatformSelect").addEventListener("change", (event) => { state.installerPlatform = event.target.value; renderInstallerHistory(); });
  document.querySelectorAll("[data-installer-mode]").forEach((button) => button.addEventListener("click", () => {
    state.installerMode = button.dataset.installerMode;
    document.querySelectorAll("[data-installer-mode]").forEach((item) => item.setAttribute("aria-pressed", String(item === button)));
    renderInstallerHistory();
  }));
  $("moreRowsButton").addEventListener("click", () => { state.rowsShown += 50; renderDaily(); });
  $("exportButton").addEventListener("click", exportCsv);
  function bindChart(svgId, chartKey) {
    $(svgId).addEventListener("pointermove", (event) => {
      const chart = state[chartKey];
      if (!chart?.rows.length) return;
      const point = event.currentTarget.createSVGPoint();
      point.x = event.clientX; point.y = event.clientY;
      const matrix = event.currentTarget.getScreenCTM();
      if (!matrix) return;
      const value = point.matrixTransform(matrix.inverse()).x;
      const index = Math.round((value - chart.x(0)) / Math.max(1, chart.x(chart.rows.length - 1) - chart.x(0)) * (chart.rows.length - 1));
      showChartPoint(index, chartKey);
    });
    $(svgId).addEventListener("pointerleave", () => hideChartPoint(chartKey));
    $(svgId).addEventListener("focus", () => showChartPoint(state[chartKey]?.index ?? 0, chartKey));
    $(svgId).addEventListener("blur", () => hideChartPoint(chartKey));
    $(svgId).addEventListener("keydown", (event) => {
      const chart = state[chartKey];
      if (!chart?.rows.length || !["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
      event.preventDefault();
      showChartPoint(event.key === "Home" ? 0 : event.key === "End" ? chart.rows.length - 1 : chart.index + (event.key === "ArrowLeft" ? -1 : 1), chartKey);
    });
  }
  bindChart("trendChart", "chart");
  bindChart("installerChart", "installerChart");
  if (typeof ResizeObserver === "function" && typeof requestAnimationFrame === "function") {
    const chartSizes = new Map();
    let resizeFrame = 0;
    const chartObserver = new ResizeObserver((entries) => {
      let changed = false;
      for (const entry of entries) {
        const size = `${Math.round(entry.contentRect.width)}:${Math.round(entry.contentRect.height)}`;
        if (chartSizes.get(entry.target) !== size) {
          chartSizes.set(entry.target, size);
          changed = true;
        }
      }
      if (!changed) return;
      if (resizeFrame) cancelAnimationFrame(resizeFrame);
      resizeFrame = requestAnimationFrame(() => {
        resizeFrame = 0;
        renderChart();
        renderInstallerHistory();
      });
    });
    chartObserver.observe($("chartWrap"));
    chartObserver.observe($("installerChartWrap"));
  }
  render();
  loadStatus();
})();
