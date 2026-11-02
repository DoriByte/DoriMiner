(() => {
  const pt = !/^en/i.test(navigator.language || '') && /^pt/i.test(navigator.language || '');
  const say = (ptText, enText) => (pt ? ptText : enText);
  const TEXT = {
    notPaired: say('Este dispositivo não está emparelhado. No Control do computador: Definições, Acesso remoto, Emparelhar um telemóvel.', 'This device is not paired. In Control on the computer: Settings, Remote access, Pair a phone.'),
    offline: say('O Control não responde: o computador pode estar desligado, sem o Tailscale ou com o Control fechado. Tento outra vez sozinho.', 'Control does not answer: the computer may be off, without Tailscale, or with Control closed. I keep trying.'),
    otherAccount: say('Este dispositivo foi emparelhado para outra conta Tailscale.', 'This device was paired for another Tailscale account.'),
    tooManyTries: say('Demasiadas tentativas falhadas. Espere uns minutos.', 'Too many failed tries. Wait a few minutes.'),
    notConfirmed: say('Não confirmou no telemóvel: não fiz nada.', 'Not confirmed on the phone: nothing was done.'),
    confirmLate: say('A confirmação no telemóvel não chegou. Tente outra vez.', 'The confirmation on the phone did not arrive. Try again.'),
    appOld: say('A aplicação não respondeu. Atualize a aplicação DoriRig.', 'The app did not answer. Update the DoriRig app.'),
    verifying: say('A verificar a aplicação…', 'Verifying the app…'),
    verifyFailed: say('A aplicação não ficou verificada', 'The app was not verified'),
    readOnly: say('Só a aplicação DoriRig verificada muda carteiras e perfis aqui.', 'Only the verified DoriRig app changes wallets and profiles here.'),
    computerOnly: say('Isto faz-se no computador.', 'This is done on the computer.'),
    restart: say('Isto muda o que está a minerar: o miner reinicia e as placas são repostas.', 'This changes what is mining: the miner restarts and the cards are put back.'),
    saveRestart: say('Guardar e reiniciar', 'Save and restart'),
    cancel: say('Cancelar', 'Cancel'),
    refused: say('o Control recusou o pedido.', 'Control refused the request.'),
    benchRunning: say('Está um benchmark a correr: tente depois de acabar.', 'A benchmark is running: try when it ends.'),
    profileInSchedule: say('Um horário ou a troca automática usa esse perfil.', 'A schedule or the automatic switch uses that profile.'),
    stale: say('A configuração mudou no computador entretanto. Volte a abrir e repita a alteração.', 'The settings changed on the computer meanwhile. Open them again and repeat the change.'),
    tooBig: say('A configuração é grande demais para enviar daqui.', 'The settings are too large to send from here.'),
    profileActive: say('Esse é o perfil em uso: escolha outro primeiro.', 'That is the profile in use: choose another first.'),
    badName: say('Nome inválido.', 'Invalid name.'),
    savedRestartFailed: say('Ficou guardado, mas o miner não reiniciou. Veja no computador.', 'It was saved, but the miner did not restart. Check on the computer.'),
    signatureClock: say('A hora do telemóvel e a do computador não batem certo.', "The phone's clock and the computer's do not agree."),
    signatureBad: say('A assinatura desta ação não confere.', "This action's signature does not check out."),
    signatureReused: say('Esta ação assinada já foi usada.', 'This signed action was already used.'),
    notVerified: say('Isto pede a aplicação DoriRig verificada.', 'This needs the verified DoriRig app.'),
    attestSigner: say('esta aplicação não está assinada com a chave da DoriByte.', 'this app is not signed with the DoriByte key.'),
    attestUnlocked: say('o bootloader do telemóvel está desbloqueado.', "the phone's bootloader is unlocked."),
    attestModified: say('o sistema do telemóvel foi alterado.', "the phone's system was changed."),
    attestNoList: say('não consegui confirmar a lista de revogações da Google; veja a internet do computador.', "could not confirm Google's revocation list; check the computer's internet."),
    alreadyVerified: say('este telemóvel já está verificado. Para mudar de chave, revogue-o no computador e emparelhe de novo.', 'this phone is already verified. To change its key, revoke it on the computer and pair again.'),
  };
  const said = (e) => (e.code ? TEXT[e.code] ?? TEXT.refused : e.message);

  const IN_APP = (() => {
    let flagged = /(?:^|[#&])app=1(?:&|$)/.test(location.hash);
    try {
      if (flagged) sessionStorage.setItem('dori-app', '1');
      else flagged = sessionStorage.getItem('dori-app') === '1';
    } catch {

    }
    return flagged && typeof window.ReactNativeWebView?.postMessage === 'function';
  })();
  const store = (() => {
    try {
      return IN_APP ? sessionStorage : localStorage;
    } catch {
      return null;
    }
  })();
  let token = /(?:^|[#&])t=([0-9a-f]{64})/.exec(location.hash)?.[1] ?? null;
  try {
    if (token) store?.setItem('dori-control-token', token);
    else token = store?.getItem('dori-control-token') ?? null;
  } catch {

  }
  if (location.hash) history.replaceState(null, '', location.pathname);
  document.documentElement.classList.toggle('in-app', IN_APP);

  const waiting = new Map();
  let nextId = 1;
  let confirmedUntil = 0;
  if (IN_APP) {
    window.__doriAuthResult = (id, ok) => {
      const done = waiting.get(id);
      waiting.delete(id);
      if (done) done(ok === true);
    };
    window.__doriAppResult = (id, result, error) => {
      const done = waiting.get(id);
      waiting.delete(id);
      if (done) done(error ? { error } : { result });
    };
  }

  function confirmInApp() {
    if (!IN_APP || Date.now() < confirmedUntil) return Promise.resolve();
    return new Promise((resolve, reject) => {
      const id = nextId++;
      const timer = setTimeout(() => {
        waiting.delete(id);
        reject(new Error(TEXT.confirmLate));
      }, 600_000);
      waiting.set(id, (ok) => {
        clearTimeout(timer);
        if (!ok) return reject(new Error(TEXT.notConfirmed));
        confirmedUntil = Date.now() + 30_000;
        return resolve();
      });
      window.ReactNativeWebView.postMessage(JSON.stringify({ type: 'dori-auth', id }));
    });
  }

  function askApp(type, data) {
    return new Promise((resolve, reject) => {
      const id = nextId++;
      const timer = setTimeout(() => {
        waiting.delete(id);
        reject(new Error(type === 'dori-attest' ? TEXT.appOld : TEXT.confirmLate));
      }, type === 'dori-attest' ? 120_000 : 600_000);
      waiting.set(id, ({ result, error }) => {
        clearTimeout(timer);
        if (error) reject(new Error(error));
        else resolve(result);
      });
      window.ReactNativeWebView.postMessage(JSON.stringify({ type, id, ...data }));
    });
  }

  const toHex = (buffer) => [...new Uint8Array(buffer)].map((b) => b.toString(16).padStart(2, '0')).join('');
  const base64url = (bytes) => btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');

  async function signedHeaders(method, path, bodyText) {
    const timestamp = String(Date.now());
    const nonce = base64url(crypto.getRandomValues(new Uint8Array(16)));
    const bodyHash = toHex(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(bodyText)));
    const signature = await askApp('dori-sign', { payload: [method.toUpperCase(), path, timestamp, nonce, bodyHash].join('\n') });
    return { 'X-Dori-Timestamp': timestamp, 'X-Dori-Nonce': nonce, 'X-Dori-Signature': signature };
  }

  let verified = false;

  async function api(method, path, body, plain = false) {
    const bodyText = body !== undefined ? JSON.stringify(body) : undefined;
    let signature = {};
    if (method !== 'GET' && !plain) {

      await confirmInApp();
      if (IN_APP && verified) signature = await signedHeaders(method, path, bodyText ?? '');
    }
    const r = await fetch(path, {
      method,
      headers: { ...(token ? { Authorization: `Bearer ${token}` } : {}), ...(body !== undefined ? { 'Content-Type': 'application/json' } : {}), ...signature },
      ...(bodyText !== undefined ? { body: bodyText } : {}),
      cache: 'no-store',
    });
    let data = null;
    try {
      data = await r.json();
    } catch {

    }
    if (!r.ok) {
      const e = new Error(data?.error || `HTTP ${r.status}`);
      e.status = r.status;
      e.code = data?.error;
      e.errors = data?.errors;
      throw e;
    }
    return data;
  }

  function cover(text) {
    let box = document.getElementById('phone-cover');
    if (text === null) {
      if (box) box.hidden = true;
      return;
    }
    if (!box) {
      box = document.createElement('div');
      box.id = 'phone-cover';
      box.className = 'phone-cover';
      const p = document.createElement('p');
      box.append(p);
      document.body.append(box);
    }
    box.firstChild.textContent = text;
    box.hidden = false;
  }

  function banner(text) {
    let box = document.getElementById('phone-banner');
    if (!box) {
      box = document.createElement('p');
      box.id = 'phone-banner';
      box.className = 'meta phone-banner';
      document.querySelector('.page')?.prepend(box);
    }
    box.textContent = text ?? '';
    box.hidden = !text;
  }

  function ask(text, yes) {
    return new Promise((resolve) => {
      const box = document.createElement('div');
      box.className = 'phone-cover';
      const card = document.createElement('div');
      card.className = 'card stack';
      const p = document.createElement('p');
      p.textContent = text;
      const row = document.createElement('div');
      row.className = 'actions';
      const button = (label, cls, value) => {
        const b = document.createElement('button');
        b.type = 'button';
        b.className = cls;
        b.textContent = label;
        b.addEventListener('click', () => {
          box.remove();
          resolve(value);
        });
        return b;
      };
      row.append(button(yes, 'btn btn-primary', true), button(TEXT.cancel, 'btn', false));
      card.append(p, row);
      box.append(card);
      document.body.append(box);
    });
  }

  function showError(e) {
    if (e.status === 401 && !/^signature/.test(e.code ?? '')) cover(TEXT.notPaired);
    else if (e.code === 'otherAccount') cover(TEXT.otherAccount);
    else if (e.status === 429) cover(TEXT.tooManyTries);
    else if (!e.status || e.status >= 500 || e.status === 421) cover(TEXT.offline);
  }

  let attestAgainAt = 0;
  let verifyNote = null;

  function drawVerify() {
    document.documentElement.classList.toggle('verified', IN_APP && verified);
    banner(verifyNote ?? (IN_APP && verified ? null : TEXT.readOnly));
    drawPhoneState(lastConfig ? JSON.parse(lastConfig) : null);
  }

  async function attestApp() {
    if (!IN_APP || verified || Date.now() < attestAgainAt) return;
    attestAgainAt = Infinity;
    verifyNote = TEXT.verifying;
    drawVerify();
    try {
      const { challenge } = await api('POST', '/api/attest/challenge', {}, true);
      const chain = await askApp('dori-attest', { challenge });
      await api('POST', '/api/attest', { chain }, true);
      verifyNote = null;
      verified = true;
      init = await api('GET', '/api/app/init');
      init.lang = phoneLang();
      drawVerify();
      pushConfig(init.config);
    } catch (e) {
      verifyNote = `${TEXT.verifyFailed}: ${said(e)}`;
      attestAgainAt = Date.now() + 60_000;
      drawVerify();
    }
  }

  const ICONS = {
    summary: ['M3 12 12 4l9 8', 'M5 10v10h5v-6h4v6h5V10'],
    profiles: ['M4 6h10', 'M18 6h2', 'M4 12h3', 'M11 12h9', 'M4 18h12', 'M20 18h0', 'M16 4v4', 'M9 10v4', 'M18 16v4'],
    settings: ['M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6z', 'M12 2v3', 'M12 19v3', 'M2 12h3', 'M19 12h3', 'M4.9 4.9l2.1 2.1', 'M17 17l2.1 2.1', 'M4.9 19.1 7 17', 'M17 7l2.1-2.1'],
  };

  function drawIcons() {
    for (const tab of document.querySelectorAll('.tab')) {
      if (tab.querySelector('svg')) continue;
      const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
      svg.setAttribute('viewBox', '0 0 24 24');
      svg.setAttribute('class', 'tab-icon');
      svg.setAttribute('aria-hidden', 'true');
      for (const d of ICONS[tab.dataset.tab] ?? []) {
        const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
        path.setAttribute('d', d);
        svg.append(path);
      }
      tab.prepend(svg);
    }
  }

  function phoneLang() {
    try {
      const chosen = localStorage.getItem('dori-control-lang');
      if (chosen === 'pt' || chosen === 'en') return chosen;
    } catch {

    }
    return pt ? 'pt' : 'en';
  }

  function drawSettings() {
    const box = document.createElement('div');
    box.id = 'phone-settings';
    box.className = 'stack';
    const card = (title, ...rest) => {
      const c = document.createElement('div');
      c.className = 'card stack';
      const h = document.createElement('h2');
      h.textContent = title;
      c.append(h, ...rest);
      return c;
    };
    const lang = document.createElement('select');
    for (const [v, label] of [['pt', 'Português'], ['en', 'English']]) {
      const o = document.createElement('option');
      o.value = v;
      o.textContent = label;
      lang.append(o);
    }
    lang.value = phoneLang();
    lang.addEventListener('change', () => {
      try {
        localStorage.setItem('dori-control-lang', lang.value);
      } catch {

      }
      location.reload();
    });
    const state = document.createElement('p');
    state.id = 'phone-state';
    const price = document.createElement('input');
    price.type = 'number';
    price.min = '0';
    price.max = '2';
    price.step = '0.01';
    price.id = 'phone-kwh';
    price.placeholder = say('€ por kWh', '€ per kWh');
    const money = document.createElement('select');
    money.id = 'phone-money';
    for (const [v, label] of [['eur', '€'], ['usd', '$']]) {
      const o = document.createElement('option');
      o.value = v;
      o.textContent = label;
      money.append(o);
    }
    const priceBox = card(say('Dinheiro', 'Money'), money, price);
    priceBox.classList.add('manage-only');
    const save = document.createElement('button');
    save.type = 'button';
    save.className = 'btn btn-primary';
    save.textContent = say('Guardar', 'Save');
    save.addEventListener('click', async () => {
      const value = price.value === '' ? null : Number(price.value);
      const now = JSON.parse(lastConfig);
      const r = await window.control.save({ ...now, currency: money.value, auto: { ...now.auto, kwhEur: value } });
      if (r.ok) {
        lastConfig = JSON.stringify(r.config);
        onConfig(r.config);
      } else if (r.errors && Object.keys(r.errors).length) banner(say('Preço inválido.', 'Invalid price.'));
    });
    priceBox.append(save);
    const nameInput = document.createElement('input');
    nameInput.type = 'text';
    nameInput.id = 'phone-machine';
    nameInput.maxLength = 40;
    const nameSave = document.createElement('button');
    nameSave.type = 'button';
    nameSave.className = 'btn btn-primary';
    nameSave.textContent = say('Guardar nome', 'Save name');
    nameSave.addEventListener('click', async () => {
      const r = await window.control.save({ ...JSON.parse(lastConfig), machineName: nameInput.value.trim() });
      if (r.ok) {
        lastConfig = JSON.stringify(r.config);
        onConfig(r.config);
      } else if (r.errors && Object.keys(r.errors).length) banner(say('Nome inválido.', 'Invalid name.'));
    });
    const nameBox = card(say('Nome desta máquina', 'Name of this machine'), nameInput, nameSave);
    nameBox.classList.add('manage-only');
    const rest = document.createElement('p');
    rest.className = 'meta';
    rest.textContent = say('As afinações das placas, os horários, a troca automática e o emparelhar de telemóveis mudam-se na janela do computador.', 'Card settings, schedules, the automatic switch and pairing phones change in the window on the computer.');
    box.append(card(say('Este telemóvel', 'This phone'), state), card(say('Língua', 'Language'), lang), nameBox, priceBox, card(say('No computador', 'On the computer'), rest));
    document.getElementById('tab-settings')?.append(box);
  }

  function drawPhoneState(config) {
    const state = document.getElementById('phone-state');
    if (state) state.textContent = verifyNote ?? (IN_APP && verified ? say('Aplicação verificada: cada mudança pede a impressão digital.', 'App verified: each change asks for the fingerprint.') : TEXT.readOnly);
    const price = document.getElementById('phone-kwh');
    if (price && document.activeElement !== price) price.value = config?.auto?.kwhEur ?? '';
    const money = document.getElementById('phone-money');
    if (money && document.activeElement !== money && config?.currency) money.value = config.currency;
    const machine = document.getElementById('phone-machine');
    if (machine && document.activeElement !== machine) machine.value = config?.machineName ?? '';
  }

  drawIcons();
  drawSettings();

  let init = null;
  let onStatus = () => undefined;
  let onConfig = () => undefined;
  let lastConfig = '';
  let saving = false;

  function pushConfig(config) {
    const text = JSON.stringify(config);
    if (text === lastConfig || saving) return;
    lastConfig = text;
    onConfig(config);
  }

  async function refresh() {
    try {
      const r = await api('GET', '/api/app/status');
      verified = r.device?.attested === true;
      cover(null);
      drawVerify();
      onStatus(r.status);

      pushConfig(r.config);
      void attestApp();
      return r.status;
    } catch (e) {
      showError(e);
      return null;
    }
  }

  const refused = (e) => {
    banner(said(e));
    return null;
  };

  const fromState = async () => {
    const r = await api('GET', '/api/app/status');
    return r.status;
  };

  window.control = {
    init: async () => {
      try {
        init = await api('GET', '/api/app/init');
      } catch (e) {
        showError(e);

        init = { config: null, coins: {}, lang: pt ? 'pt' : 'en', platform: 'phone', version: '', view: { mode: 'window', alwaysOnTop: false }, policy: {}, defaultCurve: [] };
        await new Promise(() => undefined);
      }
      verified = init.device?.attested === true;
      lastConfig = JSON.stringify(init.config);
      init.lang = phoneLang();

      setTimeout(() => {
        drawIcons();
        drawVerify();
      }, 0);
      return init;
    },
    status: async () => (await refresh()) ?? { state: 'stopped', armed: false, note: null, gpus: [], log: [], summary: null, admin: true },
    start: async () => {
      try {
        await api('POST', '/api/start', {});
      } catch (e) {
        refused(e);
      }
      return fromState();
    },
    stop: async () => {
      try {
        await api('POST', '/api/stop', {});
      } catch (e) {
        refused(e);
      }
      return fromState();
    },
    mineWithoutOc: async () => {
      try {
        await api('POST', '/api/without-oc', {});
      } catch (e) {
        refused(e);
      }
      return fromState();
    },
    save: async (next) => {
      saving = true;
      try {
        const lastCfg = JSON.parse(lastConfig || 'null') ?? init.config;
        const same = (c) => JSON.stringify({ wallets: c.wallets, pools: c.pools, profiles: c.profiles, kwh: c.auto?.kwhEur, money: c.currency, machine: c.machineName });
        if (same(next) === same(lastCfg) && next.activeProfile !== lastCfg.activeProfile) {
          await api('POST', '/api/profile', { id: next.activeProfile });
          const r = { ...next };
          lastConfig = JSON.stringify(r);
          return { ok: true, config: r, lang: init.lang };
        }
        const send = (restart) => api('POST', '/api/manage/config', { config: { rev: next.rev, wallets: next.wallets, pools: next.pools, profiles: next.profiles, activeProfile: next.activeProfile, currency: next.currency, machineName: next.machineName, auto: { kwhEur: next.auto?.kwhEur ?? null } }, ...(restart ? { restart: true } : {}) });
        let r;
        try {
          r = await send(false);
        } catch (e) {
          if (e.code !== 'restartNeeded') throw e;
          if (!(await ask(TEXT.restart, TEXT.saveRestart))) return { ok: false, errors: {} };
          r = await send(true);
        }
        lastConfig = JSON.stringify(r.config);
        return { ok: true, config: r.config, restarted: r.restarted, lang: init.lang };
      } catch (e) {
        if (e.errors) return { ok: false, errors: e.errors };
        banner(said(e));
        showError(e);
        return { ok: false, errors: {} };
      } finally {
        saving = false;
      }
    },

    view: async () => init.view,
    browseMiner: async () => null,
    elevate: async () => null,
    restoreCards: fromState,
    remoteInfo: async () => ({ enabled: false, devices: [] }),
    remoteServe: async () => ({ ok: false }),
    remotePair: async () => ({ ok: false }),
    remoteRevoke: async () => ({ enabled: false, devices: [] }),
    benchPlan: async (scope) => {
      try {
        return (await api('POST', '/api/manage/bench-plan', { scope })).plan;
      } catch (e) {
        banner(said(e));
        throw e;
      }
    },
    benchStart: async (scope, useDev) => {
      try {
        const r = await api('POST', '/api/manage/bench-start', { scope, useDev });
        return { ...r.bench, status: await fromState() };
      } catch (e) {
        banner(said(e));
        return { ok: false, code: e.code, status: await fromState() };
      }
    },
    benchCancel: async () => {
      try {
        await api('POST', '/api/manage/bench-cancel', {});
      } catch (e) {
        banner(said(e));
      }
      return fromState();
    },
    onStatus: (fn) => {
      onStatus = fn;
      setInterval(() => {
        if (!document.hidden) void refresh();
      }, 3000);
    },
    onConfig: (fn) => {
      onConfig = fn;
    },
  };
})();
