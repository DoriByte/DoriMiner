const KRYPTEX_REGIONS = [
  ['', 'Global'],
  ['-eu', 'Europe'],
  ['-us', 'North America'],
  ['-br', 'South America'],
  ['-sg', 'Singapore'],
  ['-hk', 'Hong Kong'],
  ['-ru', 'Russia'],
  ['-ae', 'Middle East'],
];

const SUPRNOVA_HOSTS = [
  ['quantus.suprnova.cc', 'Europe (Germany)', 'de'],
  ['stratum-eu2.suprnova.cc', 'Europe (Kyiv)', 'eu2'],
  ['stratum-ord.suprnova.cc', 'North America', 'us'],
  ['stratum-hkg.suprnova.cc', 'Hong Kong', 'hk'],
];

const COINS = {
  QTC: {
    id: 'QTC',
    name: 'Quantus (QTC)',
    short: 'Quantus',

    wallet: /^qz[1-9A-HJ-NP-Za-km-z]{30,60}$/,
    walletHint: 'starts with "qz"',

    pools: [
      ...KRYPTEX_REGIONS.map(([suffix, region]) => ({
        id: `kryptex-qtc${suffix}`,
        name: `Kryptex · ${region}`,
        url: `stratum+ssl://qtc${suffix}.kryptex.network:8049`,
      })),
      ...SUPRNOVA_HOSTS.map(([host, region, tag]) => ({
        id: `suprnova-qtc-${tag}`,
        name: `Suprnova · ${region}`,
        url: `stratum+ssl://${host}:7074`,
      })),

      { id: 'luckypool-qtc-eu', name: 'LuckyPool · Europe', url: 'stratum+ssl://eu.lproute.com:5660', tested: false },
      { id: 'luckypool-qtc-us', name: 'LuckyPool · North America', url: 'stratum+ssl://us.lproute.com:5660', tested: false },
      { id: 'himpool-qtc-eu', name: 'Himpool · Europe', url: 'stratum+ssl://eu.himpool.com:1336', tested: false },
      { id: 'k1pool-qtc-eu', name: 'K1Pool · Europe', url: 'stratum+ssl://eu.quantus.k1pool.com:5660', tested: false },

      { id: '2miners-qtc', name: '2Miners · Europe', url: 'stratum+ssl://qtc.2miners.com:13636' },
      { id: 'qelvpool-qtc', name: 'Qelvpool', url: 'stratum+ssl://quantus.qelvhash.com:4443' },

      { id: '2miners-qtc-us', name: '2Miners · North America', url: 'stratum+ssl://us-qtc.2miners.com:13636', tested: false },
      { id: '2miners-qtc-asia', name: '2Miners · Asia', url: 'stratum+ssl://asia-qtc.2miners.com:13636', tested: false },
    ],
    defaultPool: 'kryptex-qtc-eu',

    miner: 'quantus',
  },
  PRL: {
    id: 'PRL',
    name: 'Pearl (PRL)',
    short: 'Pearl',

    wallet: /^prl1[02-9ac-hj-np-z]{20,90}$/,
    walletHint: 'starts with "prl1"',

    pools: [
      ...KRYPTEX_REGIONS.map(([suffix, region]) => ({
        id: `kryptex-prl${suffix}`,
        name: `Kryptex · ${region}`,
        url: `stratum+ssl://prl${suffix}.kryptex.network:8048`,
      })),
      { id: 'herominers-prl', name: 'HeroMiners · Global', url: 'stratum+ssl://pearl.herominers.com:1200' },
      { id: 'suprnova-prl-de', name: 'Suprnova · Europe (Germany)', url: 'stratum+ssl://prl.suprnova.cc:3374' },
      { id: 'suprnova-prl-us', name: 'Suprnova · North America', url: 'stratum+ssl://stratum-us.suprnova.cc:3374', tested: false },
      { id: 'suprnova-prl-apac', name: 'Suprnova · Singapore', url: 'stratum+ssl://stratum-apac.suprnova.cc:3374', tested: false },
      { id: '2miners-prl', name: '2Miners · Europe', url: 'stratum+ssl://prl.2miners.com:11818' },
      { id: '2miners-prl-us', name: '2Miners · North America', url: 'stratum+ssl://us-prl.2miners.com:11818', tested: false },
      { id: '2miners-prl-asia', name: '2Miners · Asia', url: 'stratum+ssl://asia-prl.2miners.com:11818', tested: false },
      { id: 'luckypool-prl-eu1', name: 'LuckyPool · Europe 1', url: 'stratum+ssl://pearl-eu1.luckypool.io:3360' },
      { id: 'luckypool-prl-eu2', name: 'LuckyPool · Europe 2', url: 'stratum+ssl://pearl-eu2.luckypool.io:3360' },
    ],
    defaultPool: 'kryptex-prl-eu',
    miner: 'pearl',

    cards: /RTX (30|40|50)\d\d/i,
    cardsNote: 'RTX 30, 40 and 50',
  },
  VELA: {
    id: 'VELA',
    name: 'Vela (VELA)',
    short: 'Vela',

    wallet: /^(vela1[02-9ac-hj-np-z]{20,90}|[VX][1-9A-HJ-NP-Za-km-z]{25,40})$/,
    walletHint: 'starts with "vela1"',

    pools: SUPRNOVA_HOSTS.map(([host, region, tag]) => ({
      id: `suprnova-vela-${tag}`,
      name: `Suprnova · ${region}`,
      url: `stratum+ssl://${tag === 'de' ? 'vela.suprnova.cc' : host}:7604`,
      ...(tag === 'de' ? {} : { tested: false }),
    })),
    defaultPool: 'suprnova-vela-de',
    miner: 'vela',

    unit: 'inf/s',
    cards: /RTX (30|40|50)\d\d/i,
    cardsNote: 'RTX 30, 40 and 50',
  },
  ERG: {
    id: 'ERG',
    name: 'Ergo (ERG)',
    short: 'Ergo',

    wallet: /^9[1-9A-HJ-NP-Za-km-z]{50}$/,
    walletHint: 'starts with "9"',

    pools: [
      ...KRYPTEX_REGIONS.map(([suffix, region]) => ({
        id: `kryptex-erg${suffix}`,
        name: `Kryptex · ${region}`,
        url: `stratum+ssl://erg${suffix}.kryptex.network:8021`,
        ...(suffix === '-eu' ? {} : { tested: false }),
      })),
      { id: 'herominers-erg', name: 'HeroMiners · Global', url: 'stratum+ssl://ergo.herominers.com:1180' },
      ...[['de', 'Europe (Germany)'], ['fi', 'Europe (Finland)'], ['us', 'North America (US)'], ['ca', 'North America (Canada)'], ['sg', 'Singapore'], ['hk', 'Hong Kong']].map(([tag, region]) => ({
        id: `herominers-erg-${tag}`,
        name: `HeroMiners · ${region}`,
        url: `stratum+ssl://${tag}.ergo.herominers.com:1180`,
        tested: false,
      })),
      { id: '2miners-erg', name: '2Miners · Europe', url: 'stratum+ssl://erg.2miners.com:18888' },
      { id: '2miners-erg-us', name: '2Miners · North America', url: 'stratum+ssl://us-erg.2miners.com:18888', tested: false },
      { id: '2miners-erg-asia', name: '2Miners · Asia', url: 'stratum+ssl://asia-erg.2miners.com:18888', tested: false },
      { id: 'woolypooly-erg', name: 'WoolyPooly', url: 'stratum+ssl://pool.woolypooly.com:3100', tested: false },
      { id: 'lithos-erg', name: 'Lithos · this computer', url: 'stratum+tcp://127.0.0.1:4444', tested: false },
    ],
    defaultPool: 'kryptex-erg-eu',
    miner: 'ergo',

    cards: /RTX (20|30|40|50)\d\d/i,
    cardsNote: 'RTX 20, 30, 40 and 50',
  },
};

module.exports = { COINS };
