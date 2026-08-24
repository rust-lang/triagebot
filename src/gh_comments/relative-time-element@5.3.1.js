/* https://github.com/github/relative-time-element - MIT License */

var __defProp = Object.defineProperty;
var __name = (target, value) => __defProp(target, "name", { value, configurable: true });

// dist/intl-cache.js
var __classPrivateFieldGet = function(receiver, state, kind, f) {
  if (kind === "a" && !f) throw new TypeError("Private accessor was defined without a getter");
  if (typeof state === "function" ? receiver !== state || !f : !state.has(receiver)) throw new TypeError("Cannot read private member from an object whose class did not declare it");
  return kind === "m" ? f : kind === "a" ? f.call(receiver) : f ? f.value : state.get(receiver);
};
var _LRUCache_map;
var LRUCache = class {
  static {
    __name(this, "LRUCache");
  }
  constructor(max = 100) {
    this.max = max;
    _LRUCache_map.set(this, /* @__PURE__ */ new Map());
  }
  get(key) {
    const value = __classPrivateFieldGet(this, _LRUCache_map, "f").get(key);
    if (value !== void 0) {
      __classPrivateFieldGet(this, _LRUCache_map, "f").delete(key);
      __classPrivateFieldGet(this, _LRUCache_map, "f").set(key, value);
    }
    return value;
  }
  set(key, value) {
    __classPrivateFieldGet(this, _LRUCache_map, "f").delete(key);
    __classPrivateFieldGet(this, _LRUCache_map, "f").set(key, value);
    if (__classPrivateFieldGet(this, _LRUCache_map, "f").size > this.max) {
      __classPrivateFieldGet(this, _LRUCache_map, "f").delete(__classPrivateFieldGet(this, _LRUCache_map, "f").keys().next().value);
    }
  }
  clear() {
    __classPrivateFieldGet(this, _LRUCache_map, "f").clear();
  }
};
_LRUCache_map = /* @__PURE__ */ new WeakMap();
var registeredCaches = [];
function createCache(max) {
  const cache = new LRUCache(max);
  registeredCaches.push(cache);
  return cache;
}
__name(createCache, "createCache");
function clearIntlCache() {
  for (const cache of registeredCaches)
    cache.clear();
}
__name(clearIntlCache, "clearIntlCache");
if (typeof window !== "undefined" && typeof window.addEventListener === "function") {
  window.addEventListener("languagechange", clearIntlCache);
}
function cacheKey(locale, options) {
  return `${locale}\0${options ? JSON.stringify(options) : ""}`;
}
__name(cacheKey, "cacheKey");
var dateTimeFormats = createCache();
function dateTimeFormat(locale, options) {
  const key = cacheKey(locale, options);
  let format = dateTimeFormats.get(key);
  if (!format)
    dateTimeFormats.set(key, format = new Intl.DateTimeFormat(locale, options));
  return format;
}
__name(dateTimeFormat, "dateTimeFormat");
var relativeTimeFormats = createCache();
function relativeTimeFormat(locale, options) {
  const key = cacheKey(locale, options);
  let format = relativeTimeFormats.get(key);
  if (!format)
    relativeTimeFormats.set(key, format = new Intl.RelativeTimeFormat(locale, options));
  return format;
}
__name(relativeTimeFormat, "relativeTimeFormat");
var numberFormats = createCache();
function numberFormat(locale, options) {
  const key = cacheKey(locale, options);
  let format = numberFormats.get(key);
  if (!format)
    numberFormats.set(key, format = new Intl.NumberFormat(locale, options));
  return format;
}
__name(numberFormat, "numberFormat");
var locales = createCache();
function getLocale(tag) {
  let value = locales.get(tag);
  if (!value)
    locales.set(tag, value = new Intl.Locale(tag));
  return value;
}
__name(getLocale, "getLocale");

// dist/duration-format-ponyfill.js
var __classPrivateFieldSet = function(receiver, state, value, kind, f) {
  if (kind === "m") throw new TypeError("Private method is not writable");
  if (kind === "a" && !f) throw new TypeError("Private accessor was defined without a setter");
  if (typeof state === "function" ? receiver !== state || !f : !state.has(receiver)) throw new TypeError("Cannot write private member to an object whose class did not declare it");
  return kind === "a" ? f.call(receiver, value) : f ? f.value = value : state.set(receiver, value), value;
};
var __classPrivateFieldGet2 = function(receiver, state, kind, f) {
  if (kind === "a" && !f) throw new TypeError("Private accessor was defined without a getter");
  if (typeof state === "function" ? receiver !== state || !f : !state.has(receiver)) throw new TypeError("Cannot read private member from an object whose class did not declare it");
  return kind === "m" ? f : kind === "a" ? f.call(receiver) : f ? f.value : state.get(receiver);
};
var _DurationFormat_options;
var ListFormatPonyFill = class {
  static {
    __name(this, "ListFormatPonyFill");
  }
  formatToParts(members) {
    const parts = [];
    for (const value of members) {
      parts.push({ type: "element", value });
      parts.push({ type: "literal", value: ", " });
    }
    return parts.slice(0, -1);
  }
};
var ListFormat = typeof Intl !== "undefined" && Intl.ListFormat || ListFormatPonyFill;
var listFormats = createCache();
function listFormat(locale, options) {
  const key = `${locale}\0${JSON.stringify(options)}`;
  let format = listFormats.get(key);
  if (!format)
    listFormats.set(key, format = new ListFormat(locale, options));
  return format;
}
__name(listFormat, "listFormat");
var partsTable = [
  ["years", "year"],
  ["months", "month"],
  ["weeks", "week"],
  ["days", "day"],
  ["hours", "hour"],
  ["minutes", "minute"],
  ["seconds", "second"],
  ["milliseconds", "millisecond"]
];
var twoDigitFormatOptions = { minimumIntegerDigits: 2 };
var DurationFormat = class {
  static {
    __name(this, "DurationFormat");
  }
  constructor(locale, options = {}) {
    _DurationFormat_options.set(this, void 0);
    let style = String(options.style || "short");
    if (style !== "long" && style !== "short" && style !== "narrow" && style !== "digital")
      style = "short";
    let prevStyle = style === "digital" ? "numeric" : style;
    const hours = options.hours || prevStyle;
    prevStyle = hours === "2-digit" ? "numeric" : hours;
    const minutes = options.minutes || prevStyle;
    prevStyle = minutes === "2-digit" ? "numeric" : minutes;
    const seconds = options.seconds || prevStyle;
    prevStyle = seconds === "2-digit" ? "numeric" : seconds;
    const milliseconds = options.milliseconds || prevStyle;
    __classPrivateFieldSet(this, _DurationFormat_options, {
      locale,
      style,
      years: options.years || style === "digital" ? "short" : style,
      yearsDisplay: options.yearsDisplay === "always" ? "always" : "auto",
      months: options.months || style === "digital" ? "short" : style,
      monthsDisplay: options.monthsDisplay === "always" ? "always" : "auto",
      weeks: options.weeks || style === "digital" ? "short" : style,
      weeksDisplay: options.weeksDisplay === "always" ? "always" : "auto",
      days: options.days || style === "digital" ? "short" : style,
      daysDisplay: options.daysDisplay === "always" ? "always" : "auto",
      hours,
      hoursDisplay: options.hoursDisplay === "always" ? "always" : style === "digital" ? "always" : "auto",
      minutes,
      minutesDisplay: options.minutesDisplay === "always" ? "always" : style === "digital" ? "always" : "auto",
      seconds,
      secondsDisplay: options.secondsDisplay === "always" ? "always" : style === "digital" ? "always" : "auto",
      milliseconds,
      millisecondsDisplay: options.millisecondsDisplay === "always" ? "always" : "auto"
    }, "f");
  }
  resolvedOptions() {
    return __classPrivateFieldGet2(this, _DurationFormat_options, "f");
  }
  formatToParts(duration) {
    const list = [];
    const options = __classPrivateFieldGet2(this, _DurationFormat_options, "f");
    const style = options.style;
    const locale = options.locale;
    for (const [unit, nfUnit] of partsTable) {
      const value = duration[unit];
      if (options[`${unit}Display`] === "auto" && !value)
        continue;
      const unitStyle = options[unit];
      const nfOpts = unitStyle === "2-digit" ? twoDigitFormatOptions : unitStyle === "numeric" ? {} : { style: "unit", unit: nfUnit, unitDisplay: unitStyle };
      let formattedValue = numberFormat(locale, nfOpts).format(value);
      if (unit === "months" && (unitStyle === "narrow" || style === "narrow" && formattedValue.endsWith("m"))) {
        formattedValue = formattedValue.replace(/(\d+)m$/, "$1mo");
      }
      list.push(formattedValue);
    }
    return listFormat(locale, {
      type: "unit",
      style: style === "digital" ? "short" : style
    }).formatToParts(list);
  }
  format(duration) {
    return this.formatToParts(duration).map((p) => p.value).join("");
  }
};
_DurationFormat_options = /* @__PURE__ */ new WeakMap();

// dist/duration.js
var durationRe = /^[-+]?P(?:(\d+)Y)?(?:(\d+)M)?(?:(\d+)W)?(?:(\d+)D)?(?:T(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?)?$/;
var durationFormats = createCache();
var durationFormatOptionFields = [
  "style",
  "years",
  "yearsDisplay",
  "months",
  "monthsDisplay",
  "weeks",
  "weeksDisplay",
  "days",
  "daysDisplay",
  "hours",
  "hoursDisplay",
  "minutes",
  "minutesDisplay",
  "seconds",
  "secondsDisplay",
  "milliseconds",
  "millisecondsDisplay"
];
function durationFormatKey(locale, opts) {
  var _a;
  let key = locale;
  for (const field of durationFormatOptionFields) {
    key += `\0${(_a = opts[field]) !== null && _a !== void 0 ? _a : ""}`;
  }
  return key;
}
__name(durationFormatKey, "durationFormatKey");
var unitNames = ["year", "month", "week", "day", "hour", "minute", "second", "millisecond"];
var isDuration = /* @__PURE__ */ __name((str) => durationRe.test(str), "isDuration");
var Duration = class _Duration {
  static {
    __name(this, "Duration");
  }
  constructor(years = 0, months = 0, weeks = 0, days = 0, hours = 0, minutes = 0, seconds = 0, milliseconds = 0) {
    this.years = years;
    this.months = months;
    this.weeks = weeks;
    this.days = days;
    this.hours = hours;
    this.minutes = minutes;
    this.seconds = seconds;
    this.milliseconds = milliseconds;
    this.years || (this.years = 0);
    this.sign || (this.sign = Math.sign(this.years));
    this.months || (this.months = 0);
    this.sign || (this.sign = Math.sign(this.months));
    this.weeks || (this.weeks = 0);
    this.sign || (this.sign = Math.sign(this.weeks));
    this.days || (this.days = 0);
    this.sign || (this.sign = Math.sign(this.days));
    this.hours || (this.hours = 0);
    this.sign || (this.sign = Math.sign(this.hours));
    this.minutes || (this.minutes = 0);
    this.sign || (this.sign = Math.sign(this.minutes));
    this.seconds || (this.seconds = 0);
    this.sign || (this.sign = Math.sign(this.seconds));
    this.milliseconds || (this.milliseconds = 0);
    this.sign || (this.sign = Math.sign(this.milliseconds));
    this.blank = this.sign === 0;
  }
  abs() {
    return new _Duration(Math.abs(this.years), Math.abs(this.months), Math.abs(this.weeks), Math.abs(this.days), Math.abs(this.hours), Math.abs(this.minutes), Math.abs(this.seconds), Math.abs(this.milliseconds));
  }
  static from(durationLike) {
    var _a;
    if (typeof durationLike === "string") {
      const str = String(durationLike).trim();
      const factor = str.startsWith("-") ? -1 : 1;
      const parsed = (_a = str.match(durationRe)) === null || _a === void 0 ? void 0 : _a.slice(1).map((x) => (Number(x) || 0) * factor);
      if (!parsed)
        return new _Duration();
      return new _Duration(...parsed);
    } else if (typeof durationLike === "object") {
      const { years, months, weeks, days, hours, minutes, seconds, milliseconds } = durationLike;
      return new _Duration(years, months, weeks, days, hours, minutes, seconds, milliseconds);
    }
    throw new RangeError("invalid duration");
  }
  static compare(one, two) {
    const now = Date.now();
    const oneApplied = Math.abs(applyDuration(now, _Duration.from(one)).getTime() - now);
    const twoApplied = Math.abs(applyDuration(now, _Duration.from(two)).getTime() - now);
    return oneApplied > twoApplied ? -1 : oneApplied < twoApplied ? 1 : 0;
  }
  toLocaleString(locale, opts) {
    const key = durationFormatKey(locale, opts);
    let format = durationFormats.get(key);
    if (!format)
      durationFormats.set(key, format = new DurationFormat(locale, opts));
    return format.format(this);
  }
};
function applyDuration(date, duration) {
  const r = new Date(date);
  if (duration.sign < 0) {
    r.setUTCSeconds(r.getUTCSeconds() + duration.seconds);
    r.setUTCMinutes(r.getUTCMinutes() + duration.minutes);
    r.setUTCHours(r.getUTCHours() + duration.hours);
    r.setUTCDate(r.getUTCDate() + duration.weeks * 7 + duration.days);
    r.setUTCMonth(r.getUTCMonth() + duration.months);
    r.setUTCFullYear(r.getUTCFullYear() + duration.years);
  } else {
    r.setUTCFullYear(r.getUTCFullYear() + duration.years);
    r.setUTCMonth(r.getUTCMonth() + duration.months);
    r.setUTCDate(r.getUTCDate() + duration.weeks * 7 + duration.days);
    r.setUTCHours(r.getUTCHours() + duration.hours);
    r.setUTCMinutes(r.getUTCMinutes() + duration.minutes);
    r.setUTCSeconds(r.getUTCSeconds() + duration.seconds);
  }
  return r;
}
__name(applyDuration, "applyDuration");
function elapsedTime(date, precision = "second", now = Date.now()) {
  const delta = date.getTime() - now;
  if (delta === 0)
    return new Duration();
  const sign = Math.sign(delta);
  const ms = Math.abs(delta);
  const sec = Math.floor(ms / 1e3);
  const min = Math.floor(sec / 60);
  const hr = Math.floor(min / 60);
  const day = Math.floor(hr / 24);
  const month = Math.floor(day / 30);
  const year = Math.floor(month / 12);
  const i = unitNames.indexOf(precision);
  return new Duration(i >= 0 ? year * sign : 0, i >= 1 ? (month - year * 12) * sign : 0, 0, i >= 3 ? (day - month * 30) * sign : 0, i >= 4 ? (hr - day * 24) * sign : 0, i >= 5 ? (min - hr * 60) * sign : 0, i >= 6 ? (sec - min * 60) * sign : 0, i >= 7 ? (ms - sec * 1e3) * sign : 0);
}
__name(elapsedTime, "elapsedTime");
function roundToSingleUnit(duration, { relativeTo = Date.now() } = {}) {
  relativeTo = new Date(relativeTo);
  if (duration.blank)
    return duration;
  const sign = duration.sign;
  let years = Math.abs(duration.years);
  let months = Math.abs(duration.months);
  let weeks = Math.abs(duration.weeks);
  let days = Math.abs(duration.days);
  let hours = Math.abs(duration.hours);
  let minutes = Math.abs(duration.minutes);
  let seconds = Math.abs(duration.seconds);
  let milliseconds = Math.abs(duration.milliseconds);
  if (milliseconds >= 900)
    seconds += Math.round(milliseconds / 1e3);
  if (seconds || minutes || hours || days || weeks || months || years) {
    milliseconds = 0;
  }
  if (seconds >= 55)
    minutes += Math.round(seconds / 60);
  if (minutes || hours || days || weeks || months || years)
    seconds = 0;
  if (minutes >= 55)
    hours += Math.round(minutes / 60);
  if (hours || days || weeks || months || years)
    minutes = 0;
  if (days && hours >= 12)
    days += Math.round(hours / 24);
  if (!days && hours >= 21)
    days += Math.round(hours / 24);
  if (days || weeks || months || years)
    hours = 0;
  const currentYear = relativeTo.getFullYear();
  const currentMonth = relativeTo.getMonth();
  const currentDate = relativeTo.getDate();
  if (days >= 27 || years + months + days) {
    const newMonthDate = new Date(relativeTo);
    newMonthDate.setDate(1);
    newMonthDate.setMonth(currentMonth + months * sign + 1);
    newMonthDate.setDate(0);
    const monthDateCorrection = Math.max(0, currentDate - newMonthDate.getDate());
    const newDate = new Date(relativeTo);
    newDate.setFullYear(currentYear + years * sign);
    newDate.setDate(currentDate - monthDateCorrection);
    newDate.setMonth(currentMonth + months * sign);
    newDate.setDate(currentDate - monthDateCorrection + days * sign);
    const yearDiff = newDate.getFullYear() - relativeTo.getFullYear();
    const monthDiff = newDate.getMonth() - relativeTo.getMonth();
    const daysDiff = Math.abs(Math.round((Number(newDate) - Number(relativeTo)) / 864e5)) + monthDateCorrection;
    const monthsDiff = Math.abs(yearDiff * 12 + monthDiff);
    if (daysDiff < 27) {
      if (days >= 6) {
        weeks += Math.round(days / 7);
        days = 0;
      } else {
        days = daysDiff;
      }
      months = years = 0;
    } else if (monthsDiff <= 11) {
      months = monthsDiff;
      years = 0;
    } else {
      months = 0;
      years = yearDiff * sign;
    }
    if (months || years)
      days = 0;
  }
  if (years)
    months = 0;
  if (weeks >= 4)
    months += Math.round(weeks / 4);
  if (months || years)
    weeks = 0;
  if (days && weeks && !months && !years) {
    weeks += Math.round(days / 7);
    days = 0;
  }
  return new Duration(years * sign, months * sign, weeks * sign, days * sign, hours * sign, minutes * sign, seconds * sign, milliseconds * sign);
}
__name(roundToSingleUnit, "roundToSingleUnit");
function getRelativeTimeUnit(duration, opts) {
  const rounded = roundToSingleUnit(duration, opts);
  if (rounded.blank)
    return [0, "second"];
  for (const unit of unitNames) {
    if (unit === "millisecond")
      continue;
    const val = rounded[`${unit}s`];
    if (val)
      return [val, unit];
  }
  return [0, "second"];
}
__name(getRelativeTimeUnit, "getRelativeTimeUnit");

// dist/relative-time-element.js
var __classPrivateFieldGet3 = function(receiver, state, kind, f) {
  if (kind === "a" && !f) throw new TypeError("Private accessor was defined without a getter");
  if (typeof state === "function" ? receiver !== state || !f : !state.has(receiver)) throw new TypeError("Cannot read private member from an object whose class did not declare it");
  return kind === "m" ? f : kind === "a" ? f.call(receiver) : f ? f.value : state.get(receiver);
};
var __classPrivateFieldSet2 = function(receiver, state, value, kind, f) {
  if (kind === "m") throw new TypeError("Private method is not writable");
  if (kind === "a" && !f) throw new TypeError("Private accessor was defined without a setter");
  if (typeof state === "function" ? receiver !== state || !f : !state.has(receiver)) throw new TypeError("Cannot write private member to an object whose class did not declare it");
  return kind === "a" ? f.call(receiver, value) : f ? f.value = value : state.set(receiver, value), value;
};
var _RelativeTimeElement_instances;
var _RelativeTimeElement_customTitle;
var _RelativeTimeElement_updating;
var _RelativeTimeElement_lang_get;
var _RelativeTimeElement_renderRoot;
var _RelativeTimeElement_getFormattedTitle;
var _RelativeTimeElement_getExplicitThreshold;
var _RelativeTimeElement_resolveFormat;
var _RelativeTimeElement_getMicroDuration;
var _RelativeTimeElement_getMicroRelativeFormat;
var _RelativeTimeElement_getDurationFormat;
var _RelativeTimeElement_getRelativeFormat;
var _RelativeTimeElement_getDateTimeFormat;
var _RelativeTimeElement_isToday;
var _RelativeTimeElement_isCurrentYear;
var _RelativeTimeElement_getUserPreferredAbsoluteTimeFormat;
var _RelativeTimeElement_updateRenderRootContent;
var _RelativeTimeElement_shouldDisplayUserPreferredAbsoluteTime;
var _RelativeTimeElement_onRelativeTimeUpdated;
var HTMLElement = globalThis.HTMLElement || null;
var emptyDuration = new Duration();
var microEmptyDuration = new Duration(0, 0, 0, 0, 0, 1);
var RelativeTimeUpdatedEvent = class extends Event {
  static {
    __name(this, "RelativeTimeUpdatedEvent");
  }
  constructor(oldText, newText, oldTitle, newTitle) {
    super("relative-time-updated", { bubbles: true, composed: true });
    this.oldText = oldText;
    this.newText = newText;
    this.oldTitle = oldTitle;
    this.newTitle = newTitle;
  }
};
function getUnitFactor(el) {
  const date = el.date;
  if (!date)
    return Infinity;
  const now = Date.now();
  if (el.format === "duration" || el.format === "elapsed") {
    const precision = el.precision;
    if (precision === "second") {
      return 1e3;
    } else if (precision === "minute") {
      return 60 * 1e3;
    }
  }
  const ms = Math.abs(now - date.getTime());
  let factor = 60 * 60 * 1e3;
  if (ms < 60 * 1e3) {
    factor = 1e3;
  } else if (ms < 60 * 60 * 1e3) {
    factor = 60 * 1e3;
  }
  const threshold = getExplicitThreshold(el);
  if (el.format === "micro" && threshold) {
    const thresholdDuration = Duration.from(threshold);
    const signedThresholdDuration = date.getTime() > now ? negateDuration(thresholdDuration) : thresholdDuration;
    const thresholdTime = applyDuration(date, signedThresholdDuration).getTime();
    const msUntilThreshold = thresholdTime - now;
    if (msUntilThreshold > 0)
      factor = Math.min(factor, msUntilThreshold);
  }
  return factor;
}
__name(getUnitFactor, "getUnitFactor");
function negateDuration(duration) {
  return new Duration(-duration.years, -duration.months, -duration.weeks, -duration.days, -duration.hours, -duration.minutes, -duration.seconds, -duration.milliseconds);
}
__name(negateDuration, "negateDuration");
function getExplicitThreshold(el) {
  const threshold = el.getAttribute("threshold");
  return threshold && isDuration(threshold) ? threshold : null;
}
__name(getExplicitThreshold, "getExplicitThreshold");
var browser12hCycle;
function isBrowser12hCycle() {
  if (browser12hCycle === void 0) {
    try {
      browser12hCycle = new Intl.DateTimeFormat([], { hour: "numeric" }).resolvedOptions().hour12 === true;
    } catch (_a) {
      browser12hCycle = false;
    }
  }
  return browser12hCycle;
}
__name(isBrowser12hCycle, "isBrowser12hCycle");
if (typeof window !== "undefined" && typeof window.addEventListener === "function") {
  window.addEventListener("languagechange", () => {
    browser12hCycle = void 0;
  });
}
var dateObserver = new class {
  constructor() {
    this.elements = /* @__PURE__ */ new Set();
    this.time = Infinity;
    this.updating = false;
    this.timer = -1;
  }
  observe(element) {
    this.elements.add(element);
    if (this.updating)
      return;
    const date = element.date;
    if (date && date.getTime()) {
      const ms = getUnitFactor(element);
      const time = Date.now() + ms;
      if (time < this.time || this.time <= Date.now()) {
        clearTimeout(this.timer);
        this.timer = setTimeout(() => this.update(), ms);
        this.time = time;
      }
    }
  }
  unobserve(element) {
    if (!this.elements.has(element))
      return;
    this.elements.delete(element);
  }
  update() {
    clearTimeout(this.timer);
    if (!this.elements.size)
      return;
    let nearestDistance = Infinity;
    this.updating = true;
    try {
      for (const timeEl of this.elements) {
        nearestDistance = Math.min(nearestDistance, getUnitFactor(timeEl));
        timeEl.update();
      }
    } finally {
      this.updating = false;
    }
    this.time = Math.min(60 * 60 * 1e3, nearestDistance);
    this.timer = setTimeout(() => this.update(), this.time);
    this.time += Date.now();
  }
}();
var RelativeTimeElement = class extends HTMLElement {
  static {
    __name(this, "RelativeTimeElement");
  }
  constructor() {
    super(...arguments);
    _RelativeTimeElement_instances.add(this);
    _RelativeTimeElement_customTitle.set(this, false);
    _RelativeTimeElement_updating.set(this, false);
    _RelativeTimeElement_renderRoot.set(this, this.shadowRoot ? this.shadowRoot : this.attachShadow ? this.attachShadow({ mode: "open" }) : this);
    _RelativeTimeElement_onRelativeTimeUpdated.set(this, null);
  }
  static define(tag = "relative-time", registry = customElements) {
    registry.define(tag, this);
    return this;
  }
  get timeZone() {
    var _a;
    const tz = ((_a = this.closest("[time-zone]")) === null || _a === void 0 ? void 0 : _a.getAttribute("time-zone")) || this.ownerDocument.documentElement.getAttribute("time-zone");
    return tz || void 0;
  }
  get hourCycle() {
    var _a;
    const hc = ((_a = this.closest("[hour-cycle]")) === null || _a === void 0 ? void 0 : _a.getAttribute("hour-cycle")) || this.ownerDocument.documentElement.getAttribute("hour-cycle");
    if (hc === "h11" || hc === "h12" || hc === "h23" || hc === "h24")
      return hc;
    return isBrowser12hCycle() ? "h12" : "h23";
  }
  static get observedAttributes() {
    return [
      "second",
      "minute",
      "hour",
      "weekday",
      "day",
      "month",
      "year",
      "time-zone-name",
      "prefix",
      "threshold",
      "tense",
      "precision",
      "format",
      "format-style",
      "no-title",
      "datetime",
      "lang",
      "title",
      "aria-hidden",
      "time-zone",
      "hour-cycle"
    ];
  }
  get onRelativeTimeUpdated() {
    return __classPrivateFieldGet3(this, _RelativeTimeElement_onRelativeTimeUpdated, "f");
  }
  set onRelativeTimeUpdated(listener) {
    if (__classPrivateFieldGet3(this, _RelativeTimeElement_onRelativeTimeUpdated, "f")) {
      this.removeEventListener("relative-time-updated", __classPrivateFieldGet3(this, _RelativeTimeElement_onRelativeTimeUpdated, "f"));
    }
    __classPrivateFieldSet2(this, _RelativeTimeElement_onRelativeTimeUpdated, typeof listener === "object" || typeof listener === "function" ? listener : null, "f");
    if (typeof listener === "function") {
      this.addEventListener("relative-time-updated", listener);
    }
  }
  get second() {
    const second = this.getAttribute("second");
    if (second === "numeric" || second === "2-digit")
      return second;
  }
  set second(value) {
    this.setAttribute("second", value || "");
  }
  get minute() {
    const minute = this.getAttribute("minute");
    if (minute === "numeric" || minute === "2-digit")
      return minute;
  }
  set minute(value) {
    this.setAttribute("minute", value || "");
  }
  get hour() {
    const hour = this.getAttribute("hour");
    if (hour === "numeric" || hour === "2-digit")
      return hour;
  }
  set hour(value) {
    this.setAttribute("hour", value || "");
  }
  get weekday() {
    const weekday = this.getAttribute("weekday");
    if (weekday === "long" || weekday === "short" || weekday === "narrow") {
      return weekday;
    }
    if (this.format === "datetime" && weekday !== "")
      return this.formatStyle;
  }
  set weekday(value) {
    this.setAttribute("weekday", value || "");
  }
  get day() {
    var _a;
    const day = (_a = this.getAttribute("day")) !== null && _a !== void 0 ? _a : "numeric";
    if (day === "numeric" || day === "2-digit")
      return day;
  }
  set day(value) {
    this.setAttribute("day", value || "");
  }
  get month() {
    const format = this.format;
    let month = this.getAttribute("month");
    if (month === "")
      return;
    month !== null && month !== void 0 ? month : month = format === "datetime" ? this.formatStyle : "short";
    if (month === "numeric" || month === "2-digit" || month === "short" || month === "long" || month === "narrow") {
      return month;
    }
  }
  set month(value) {
    this.setAttribute("month", value || "");
  }
  get year() {
    var _a;
    const year = this.getAttribute("year");
    if (year === "numeric" || year === "2-digit")
      return year;
    if (!this.hasAttribute("year") && (/* @__PURE__ */ new Date()).getUTCFullYear() !== ((_a = this.date) === null || _a === void 0 ? void 0 : _a.getUTCFullYear())) {
      return "numeric";
    }
  }
  set year(value) {
    this.setAttribute("year", value || "");
  }
  get timeZoneName() {
    const name = this.getAttribute("time-zone-name");
    if (name === "long" || name === "short" || name === "shortOffset" || name === "longOffset" || name === "shortGeneric" || name === "longGeneric") {
      return name;
    }
  }
  set timeZoneName(value) {
    this.setAttribute("time-zone-name", value || "");
  }
  get prefix() {
    var _a;
    return (_a = this.getAttribute("prefix")) !== null && _a !== void 0 ? _a : this.format === "datetime" ? "" : "on";
  }
  set prefix(value) {
    this.setAttribute("prefix", value);
  }
  get threshold() {
    const threshold = this.getAttribute("threshold");
    return threshold && isDuration(threshold) ? threshold : "P30D";
  }
  set threshold(value) {
    this.setAttribute("threshold", value);
  }
  get tense() {
    const tense = this.getAttribute("tense");
    if (tense === "past")
      return "past";
    if (tense === "future")
      return "future";
    return "auto";
  }
  set tense(value) {
    this.setAttribute("tense", value);
  }
  get precision() {
    const precision = this.getAttribute("precision");
    if (unitNames.includes(precision))
      return precision;
    if (this.format === "micro")
      return "minute";
    return "second";
  }
  set precision(value) {
    this.setAttribute("precision", value);
  }
  get format() {
    const format = this.getAttribute("format");
    if (format === "datetime")
      return "datetime";
    if (format === "relative")
      return "relative";
    if (format === "duration")
      return "duration";
    if (format === "micro")
      return "micro";
    if (format === "elapsed")
      return "elapsed";
    return "auto";
  }
  set format(value) {
    this.setAttribute("format", value);
  }
  get formatStyle() {
    const formatStyle = this.getAttribute("format-style");
    if (formatStyle === "long")
      return "long";
    if (formatStyle === "short")
      return "short";
    if (formatStyle === "narrow")
      return "narrow";
    const format = this.format;
    if (format === "elapsed" || format === "micro")
      return "narrow";
    if (format === "datetime")
      return "short";
    return "long";
  }
  set formatStyle(value) {
    this.setAttribute("format-style", value);
  }
  get noTitle() {
    return this.hasAttribute("no-title");
  }
  set noTitle(value) {
    this.toggleAttribute("no-title", value);
  }
  get datetime() {
    return this.getAttribute("datetime") || "";
  }
  set datetime(value) {
    this.setAttribute("datetime", value);
  }
  get date() {
    const parsed = Date.parse(this.datetime);
    return Number.isNaN(parsed) ? null : new Date(parsed);
  }
  set date(value) {
    this.datetime = (value === null || value === void 0 ? void 0 : value.toISOString()) || "";
  }
  connectedCallback() {
    this.update();
  }
  disconnectedCallback() {
    dateObserver.unobserve(this);
  }
  attributeChangedCallback(attrName, oldValue, newValue) {
    if (oldValue === newValue)
      return;
    if (attrName === "title") {
      __classPrivateFieldSet2(this, _RelativeTimeElement_customTitle, newValue !== null && (this.date && __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getFormattedTitle).call(this, this.date, __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "a", _RelativeTimeElement_lang_get), this.timeZone, this.hourCycle)) !== newValue, "f");
    }
    if (!__classPrivateFieldGet3(this, _RelativeTimeElement_updating, "f") && !(attrName === "title" && __classPrivateFieldGet3(this, _RelativeTimeElement_customTitle, "f"))) {
      __classPrivateFieldSet2(this, _RelativeTimeElement_updating, (async () => {
        await Promise.resolve();
        this.update();
        __classPrivateFieldSet2(this, _RelativeTimeElement_updating, false, "f");
      })(), "f");
    }
  }
  update() {
    const oldText = __classPrivateFieldGet3(this, _RelativeTimeElement_renderRoot, "f").textContent || this.textContent || "";
    const oldTitle = this.getAttribute("title") || "";
    let newTitle = oldTitle;
    const date = this.date;
    if (typeof Intl === "undefined" || !Intl.DateTimeFormat || !date) {
      __classPrivateFieldGet3(this, _RelativeTimeElement_renderRoot, "f").textContent = oldText;
      return;
    }
    const now = Date.now();
    const locale = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "a", _RelativeTimeElement_lang_get);
    const timeZone = this.timeZone;
    const hourCycle = this.hourCycle;
    if (!__classPrivateFieldGet3(this, _RelativeTimeElement_customTitle, "f")) {
      newTitle = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getFormattedTitle).call(this, date, locale, timeZone, hourCycle) || "";
      if (newTitle && !this.noTitle && newTitle !== oldTitle)
        this.setAttribute("title", newTitle);
    }
    const duration = elapsedTime(date, this.precision, now);
    const format = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_resolveFormat).call(this, duration, elapsedTime(date, "millisecond", now));
    let newText = oldText;
    const displayUserPreferredAbsoluteTime = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_shouldDisplayUserPreferredAbsoluteTime).call(this, format);
    if (displayUserPreferredAbsoluteTime) {
      newText = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getUserPreferredAbsoluteTimeFormat).call(this, date, locale, timeZone, hourCycle);
    } else {
      if (format === "duration") {
        if (this.format === "micro" && this.tense !== "auto" && Intl.RelativeTimeFormat) {
          newText = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getMicroRelativeFormat).call(this, duration, locale);
        } else {
          newText = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getDurationFormat).call(this, duration, locale);
        }
      } else if (format === "relative") {
        newText = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getRelativeFormat).call(this, duration, locale);
      } else {
        newText = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getDateTimeFormat).call(this, date, locale, timeZone, hourCycle);
      }
    }
    if (newText) {
      __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_updateRenderRootContent).call(this, newText);
    } else if (this.shadowRoot === __classPrivateFieldGet3(this, _RelativeTimeElement_renderRoot, "f") && this.textContent) {
      __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_updateRenderRootContent).call(this, this.textContent);
    }
    if (newText !== oldText || newTitle !== oldTitle) {
      this.dispatchEvent(new RelativeTimeUpdatedEvent(oldText, newText, oldTitle, newTitle));
    }
    const shouldObserve = !displayUserPreferredAbsoluteTime && (format === "relative" || format === "duration") || this.format === "micro" && Boolean(__classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getExplicitThreshold).call(this)) && date.getTime() > now || displayUserPreferredAbsoluteTime && (__classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_isToday).call(this, date, locale, timeZone) || __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_isCurrentYear).call(this, date, locale, timeZone));
    if (shouldObserve) {
      dateObserver.observe(this);
    } else {
      dateObserver.unobserve(this);
    }
  }
};
_RelativeTimeElement_customTitle = /* @__PURE__ */ new WeakMap(), _RelativeTimeElement_updating = /* @__PURE__ */ new WeakMap(), _RelativeTimeElement_renderRoot = /* @__PURE__ */ new WeakMap(), _RelativeTimeElement_onRelativeTimeUpdated = /* @__PURE__ */ new WeakMap(), _RelativeTimeElement_instances = /* @__PURE__ */ new WeakSet(), _RelativeTimeElement_lang_get = /* @__PURE__ */ __name(function _RelativeTimeElement_lang_get2() {
  var _a;
  const lang = ((_a = this.closest("[lang]")) === null || _a === void 0 ? void 0 : _a.getAttribute("lang")) || this.ownerDocument.documentElement.getAttribute("lang");
  if (!lang)
    return "default";
  try {
    return getLocale(lang).toString();
  } catch (_b) {
    return "default";
  }
}, "_RelativeTimeElement_lang_get"), _RelativeTimeElement_getFormattedTitle = /* @__PURE__ */ __name(function _RelativeTimeElement_getFormattedTitle2(date, locale, timeZone, hourCycle) {
  return dateTimeFormat(locale, {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
    timeZoneName: "short",
    timeZone,
    hourCycle
  }).format(date);
}, "_RelativeTimeElement_getFormattedTitle"), _RelativeTimeElement_getExplicitThreshold = /* @__PURE__ */ __name(function _RelativeTimeElement_getExplicitThreshold2() {
  return getExplicitThreshold(this);
}, "_RelativeTimeElement_getExplicitThreshold"), _RelativeTimeElement_resolveFormat = /* @__PURE__ */ __name(function _RelativeTimeElement_resolveFormat2(duration, thresholdDuration = duration) {
  const format = this.format;
  if (format === "datetime")
    return "datetime";
  if (format === "duration")
    return "duration";
  if (format === "elapsed")
    return "duration";
  if (format === "micro") {
    const threshold = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getExplicitThreshold).call(this);
    if (threshold && Duration.compare(thresholdDuration, threshold) === -1)
      return "datetime";
    return "duration";
  }
  if ((format === "auto" || format === "relative") && typeof Intl !== "undefined" && Intl.RelativeTimeFormat) {
    const tense = this.tense;
    if (tense === "past" || tense === "future")
      return "relative";
    if (Duration.compare(duration, this.threshold) === 1)
      return "relative";
  }
  return "datetime";
}, "_RelativeTimeElement_resolveFormat"), _RelativeTimeElement_getMicroDuration = /* @__PURE__ */ __name(function _RelativeTimeElement_getMicroDuration2(duration) {
  duration = roundToSingleUnit(duration);
  if (duration.months === 0 && (this.tense === "past" && duration.sign !== -1 || this.tense === "future" && duration.sign !== 1)) {
    return microEmptyDuration;
  }
  return duration;
}, "_RelativeTimeElement_getMicroDuration"), _RelativeTimeElement_getMicroRelativeFormat = /* @__PURE__ */ __name(function _RelativeTimeElement_getMicroRelativeFormat2(duration, locale) {
  const relativeFormat = relativeTimeFormat(locale, {
    numeric: "always",
    style: "narrow"
  });
  duration = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getMicroDuration).call(this, duration);
  const [int, unit] = getRelativeTimeUnit(duration.blank ? microEmptyDuration : duration);
  return relativeFormat.format(Math.abs(int) * (this.tense === "past" ? -1 : 1), unit);
}, "_RelativeTimeElement_getMicroRelativeFormat"), _RelativeTimeElement_getDurationFormat = /* @__PURE__ */ __name(function _RelativeTimeElement_getDurationFormat2(duration, locale) {
  const format = this.format;
  const style = this.formatStyle;
  const tense = this.tense;
  let empty = emptyDuration;
  if (format === "micro") {
    duration = __classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_getMicroDuration).call(this, duration);
    empty = microEmptyDuration;
  } else if (tense === "past" && duration.sign !== -1 || tense === "future" && duration.sign !== 1) {
    duration = empty;
  }
  const display = `${this.precision}sDisplay`;
  if (duration.blank) {
    return empty.toLocaleString(locale, { style, [display]: "always" });
  }
  return duration.abs().toLocaleString(locale, { style });
}, "_RelativeTimeElement_getDurationFormat"), _RelativeTimeElement_getRelativeFormat = /* @__PURE__ */ __name(function _RelativeTimeElement_getRelativeFormat2(duration, locale) {
  const relativeFormat = relativeTimeFormat(locale, {
    numeric: "auto",
    style: this.formatStyle
  });
  const tense = this.tense;
  if (tense === "future" && duration.sign !== 1)
    duration = emptyDuration;
  if (tense === "past" && duration.sign !== -1)
    duration = emptyDuration;
  const [int, unit] = getRelativeTimeUnit(duration);
  if (unit === "second" && int < 10) {
    return relativeFormat.format(0, this.precision === "millisecond" ? "second" : this.precision);
  }
  return relativeFormat.format(int, unit);
}, "_RelativeTimeElement_getRelativeFormat"), _RelativeTimeElement_getDateTimeFormat = /* @__PURE__ */ __name(function _RelativeTimeElement_getDateTimeFormat2(date, locale, timeZone, hourCycle) {
  const formatter = dateTimeFormat(locale, {
    second: this.second,
    minute: this.minute,
    hour: this.hour,
    weekday: this.weekday,
    day: this.day,
    month: this.month,
    year: this.year,
    timeZoneName: this.timeZoneName,
    timeZone,
    hourCycle
  });
  return `${this.prefix} ${formatter.format(date)}`.trim();
}, "_RelativeTimeElement_getDateTimeFormat"), _RelativeTimeElement_isToday = /* @__PURE__ */ __name(function _RelativeTimeElement_isToday2(date, locale, timeZone) {
  const now = /* @__PURE__ */ new Date();
  const formatter = dateTimeFormat(locale, {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit"
  });
  return formatter.format(now) === formatter.format(date);
}, "_RelativeTimeElement_isToday"), _RelativeTimeElement_isCurrentYear = /* @__PURE__ */ __name(function _RelativeTimeElement_isCurrentYear2(date, locale, timeZone) {
  const now = /* @__PURE__ */ new Date();
  const formatter = dateTimeFormat(locale, {
    timeZone,
    year: "numeric"
  });
  return formatter.format(now) === formatter.format(date);
}, "_RelativeTimeElement_isCurrentYear"), _RelativeTimeElement_getUserPreferredAbsoluteTimeFormat = /* @__PURE__ */ __name(function _RelativeTimeElement_getUserPreferredAbsoluteTimeFormat2(date, locale, timeZone, hourCycle) {
  const timeOnlyOptions = {
    hour: "numeric",
    minute: "2-digit",
    timeZoneName: "short",
    timeZone,
    hourCycle
  };
  if (__classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_isToday).call(this, date, locale, timeZone)) {
    const relativeFormatter = relativeTimeFormat(locale, { numeric: "auto" });
    let todayText = relativeFormatter.format(0, "day");
    todayText = todayText.charAt(0).toLocaleUpperCase(locale) + todayText.slice(1);
    const timeOnly = dateTimeFormat(locale, timeOnlyOptions).format(date);
    return `${todayText} ${timeOnly}`;
  }
  const timeAndDateOptions = Object.assign(Object.assign({}, timeOnlyOptions), { day: "numeric", month: "short" });
  if (__classPrivateFieldGet3(this, _RelativeTimeElement_instances, "m", _RelativeTimeElement_isCurrentYear).call(this, date, locale, timeZone)) {
    return dateTimeFormat(locale, timeAndDateOptions).format(date);
  }
  return dateTimeFormat(locale, Object.assign(Object.assign({}, timeAndDateOptions), { year: "numeric" })).format(date);
}, "_RelativeTimeElement_getUserPreferredAbsoluteTimeFormat"), _RelativeTimeElement_updateRenderRootContent = /* @__PURE__ */ __name(function _RelativeTimeElement_updateRenderRootContent2(content) {
  const root2 = __classPrivateFieldGet3(this, _RelativeTimeElement_renderRoot, "f");
  const ariaHidden = this.hasAttribute("aria-hidden") && this.getAttribute("aria-hidden") === "true";
  let span = root2.firstElementChild;
  if (!span || span.getAttribute("part") !== "root" || root2.childNodes.length !== 1) {
    span = document.createElement("span");
    span.setAttribute("part", "root");
    root2.replaceChildren(span);
  }
  if (ariaHidden) {
    if (span.getAttribute("aria-hidden") !== "true")
      span.setAttribute("aria-hidden", "true");
  } else if (span.hasAttribute("aria-hidden")) {
    span.removeAttribute("aria-hidden");
  }
  if (span.textContent !== content) {
    span.textContent = content;
  }
}, "_RelativeTimeElement_updateRenderRootContent"), _RelativeTimeElement_shouldDisplayUserPreferredAbsoluteTime = /* @__PURE__ */ __name(function _RelativeTimeElement_shouldDisplayUserPreferredAbsoluteTime2(format) {
  var _a;
  if (format === "duration" && this.format !== "micro")
    return false;
  return this.ownerDocument.documentElement.getAttribute("data-prefers-absolute-time") === "true" || ((_a = this.ownerDocument.body) === null || _a === void 0 ? void 0 : _a.getAttribute("data-prefers-absolute-time")) === "true";
}, "_RelativeTimeElement_shouldDisplayUserPreferredAbsoluteTime");

// dist/relative-time-element-define.js
var root = typeof globalThis !== "undefined" ? globalThis : window;
try {
  root.RelativeTimeElement = RelativeTimeElement.define();
} catch (e) {
  if (!(root.DOMException && e instanceof DOMException && e.name === "NotSupportedError") && !(e instanceof ReferenceError)) {
    throw e;
  }
}
var relative_time_element_define_default = RelativeTimeElement;

// dist/index.js
var index_default = relative_time_element_define_default;
export {
  relative_time_element_define_default as RelativeTimeElement,
  RelativeTimeUpdatedEvent,
  index_default as default
};
