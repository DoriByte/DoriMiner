'use strict';

const T = {
  pt: {
    tabSummary: 'Resumo',
    tabProfiles: 'Perfis',
    tabSchedule: 'Horários',
    tabSettings: 'Definições',
    hashrate: 'Hashrate',
    shares: 'Shares aceites',
    pool: 'Pool',
    uptime: 'A minerar há',
    energy: 'Desde o arranque',
    energyToday2: 'Hoje',
    energy30: '30 dias',
    income: 'Rendimento',
    currency: 'Moeda',
    machineName: 'Nome desta máquina',
    machineNameHint: 'Aparece aqui e no telemóvel. Vazio, fica o do sistema. Não muda o worker dos perfis.',
    err_badMachineName: 'O nome pode ter até 40 caracteres, sem caracteres de controlo.',
    segWallets: 'Carteiras',
    segProfiles: 'Perfis',
    segPools: 'Pools',
    knownPools: 'Pools conhecidas',
    poolTested: 'Testada',
    poolUntested: 'Por testar',
    addPool: 'Nova pool',
    addPoolHint: 'Qualquer pool de {coin}: indique o nome e o endereço.',
    benchTitle: 'Benchmark',
    benchNone: 'Ainda não foi feito.',
    energyMining: 'a minerar',
    energyToday: 'hoje',
    power: 'Consumo',
    perWatt: 'por watt',
    minerOutput: 'O que o miner escreve',
    profile: 'Perfil',
    coin: 'Moeda',
    worker: 'Nome desta máquina (worker)',
    wallet: 'Carteira',
    wallets: 'Carteiras',
    walletHintQTC: 'Começa por "qz". Só se guarda neste computador.',
    walletHintPRL: 'Começa por "prl1". Só se guarda neste computador. A Pearl mina-se em placas RTX 30, 40 e 50.',
    walletHintVELA: 'Começa por "vela1". Só se guarda neste computador. A Vela mina-se em placas RTX 30, 40 e 50; o miner descarrega o modelo (1,2 GB) no primeiro arranque.',
    walletHintERG: 'Começa por "9". Só se guarda neste computador. A Ergo mina-se em placas RTX 20, 30, 40 e 50 com 8 GB ou mais. No Lithos quem paga é o teu nó.',
    usedByMiner: 'Em uso por outro miner: {name}',
    usedByProgram: 'Em uso por outro programa: {name}',
    usedByUnknown: 'Em uso por outro programa',
    noticeErgTable: 'A tabela da Ergo já não cabe toda na memória desta placa: continua a minerar, mais devagar.',
    pools: 'Pools',
    name: 'Nome',
    address: 'Endereço',
    devFee: 'Apoio ao DoriMiner (%)',
    devFeeHint: '1 % do tempo mina para a carteira da DoriByte, na mesma pool. 0 desliga.',
    devices: 'Placas',
    devicesHint: 'Vazio: todas. Ou os números do nvidia-smi, separados por vírgulas.',
    showWindow: 'Mostrar a janela do miner',
    showWindowHint: 'A janela do próprio miner, com o banner e os relatórios. Escondida, o que ele escreve aparece no Resumo.',
    startWithControl: 'Minerar quando o Control abre',
    startWithControlHint: 'Dentro dos horários, se estiverem ligados.',
    restartOnCrash: 'Voltar a lançar o miner se ele parar sozinho',
    advanced: 'Avançado',
    minerPath: 'Onde está o dori-miner',
    browse: 'Procurar…',
    apiPort: 'Porta da API do miner',
    language: 'Língua',
    langAuto: 'A do sistema',
    save: 'Guardar',
    cancel: 'Cancelar',
    edit: 'Editar',
    remove: 'Tirar',
    use: 'Usar',
    active: 'Ativo',
    saved: 'Guardado.',
    savedRestarted: 'Guardado. O miner voltou a arrancar com as definições novas.',
    fixErrors: 'Há campos por corrigir.',
    newProfile: 'Novo perfil',
    newWallet: 'Nova carteira',
    newPool: 'Nova pool',
    addPool: 'Acrescentar pool…',
    walletNew: '+ Nova carteira…',
    noWallet: '(sem carteira)',
    poolOrder: 'Por ordem: se a primeira ficar 2 minutos sem responder, passa à seguinte, e volta à primeira de hora a hora.',
    up: 'Subir',
    down: 'Descer',
    cardsSection: 'Placas',
    cardsHint: 'Vazio = não mexer. Mudar as placas precisa de abrir o Control como administrador; tudo volta ao que estava quando o miner para.',
    powerW: 'Limite de potência (W)',
    coreLock: 'Core fixo (MHz)',
    memLock: 'Memória fixa (MHz)',
    coreOffset: 'Offset do core (MHz)',
    memOffset: 'Offset da memória (MHz)',
    fans: 'Ventoinhas',
    fanAuto: 'Automático da placa',
    fanFixed: 'Fixas',
    fanCurve: 'Curva',
    fanPct: 'Percentagem',
    curveHint: 'Temperatura (°C) → ventoinha (%). Olha para o core, ou para a memória 20 °C abaixo quando ela aquece mais.',
    addPoint: 'Acrescentar ponto',
    limitsSection: 'Limites de temperatura',
    limitsHint: 'Perto do limite as ventoinhas vão a 100 %; um minuto acima, o miner para e volta quando a placa arrefecer.',
    coreLimit: 'Core (°C)',
    memLimit: 'Memória (°C)',
    walletsHint: 'Ficam só neste computador.',
    poolsHint: 'As pools da Kryptex e da Suprnova já estão na lista dos perfis. Aqui acrescentam-se outras.',
    inUse: 'em uso',
    scheduleOn: 'Minerar só a certas horas',
    scheduleHint: 'Com o botão ligado, o Control arranca e para o miner nestas horas, e pode mudar de perfil. Um horário que acaba antes de começar passa a meia-noite.',
    addSlot: 'Acrescentar horário',
    activeProfile: 'Perfil ativo',
    from: 'das',
    to: 'às',
    days: ['Dom', 'Seg', 'Ter', 'Qua', 'Qui', 'Sex', 'Sáb'],
    stMining: 'A minerar',
    stStopped: 'Parado',
    stWaiting: 'À espera do horário',
    stRestarting: 'A recomeçar',
    stExternal: 'Miner lançado fora do Control',
    stCooling: 'A arrefecer',
    stNeedAdmin: 'Precisa de administrador',
    connected: 'ligada',
    disconnected: 'sem ligação',
    rejected: 'rejeitadas',
    stale: 'atrasadas',
    devFeeNow: 'a minerar o apoio agora',
    miner: 'Miner',
    notFound: 'não encontrado',
    found: 'encontrado',
    temp: 'Núcleo',
    memTemp: 'Memória',
    fan: 'Ventoinha',
    clock: 'Relógio',
    load: 'Carga',
    rescue: 'socorro',
    noGpu: 'O nvidia-smi não respondeu: sem leituras das placas.',
    auto: 'automático',
    other: 'Outra…',
    pin: 'Manter à frente das outras janelas',
    unpin: 'Deixar de estar sempre à frente',
    toWindow: 'Abrir em janela grande',
    toCompact: 'Modo compacto (ao lado do ecrã)',
    elevate: 'Abrir como administrador',
    withoutOc: 'Minerar sem mexer nas placas',
    restoreCards: 'Repor as placas',
    ocOk: 'Placas: perfil aplicado',
    ocPartial: 'Placas: memória não fixou',
    ocNone: 'Placas: sem alterações',
    adminYes: 'O Control está como administrador: pode mudar as placas.',
    adminNo: 'O Control não está como administrador: os perfis que mudam as placas pedem para o abrir assim.',
    err_unknownCoin: 'Moeda desconhecida.',
    err_badWallet: 'Não é um endereço desta moeda.',
    err_badPoolUrl: 'Use [stratum+tcp:// ou stratum+ssl://]endereço:porta.',
    err_unknownPool: 'Pool desconhecida ou de outra moeda.',
    err_badPools: 'De 1 a 5 pools.',
    err_badWorker: 'Letras, algarismos, - e _ (até 32).',
    err_badDevFee: 'De 0 a 5.',
    err_badDevices: 'Números das placas separados por vírgulas, por exemplo 0,1.',
    err_badMinerPath: 'Um caminho completo, sem % nem aspas.',
    err_badFlag: 'Valor inválido nas definições.',
    err_badApiPort: 'De 1024 a 65535.',
    err_badSlot: 'Escolha os dias e duas horas diferentes.',
    err_badName: 'Dê-lhe um nome (até 40 letras).',
    err_badId: 'Identificador repetido.',
    err_unknownWallet: 'Carteira desconhecida.',
    err_walletOtherCoin: 'Esta carteira é de outra moeda.',
    err_unknownProfile: 'Perfil desconhecido.',
    err_badPowerW: 'Limite de potência de 50 a 1000 W.',
    err_badCoreLock: 'Core fixo de 300 a 4000 MHz.',
    err_badMemLock: 'Memória fixa de 100 a 20000 MHz.',
    err_badCoreOffset: 'Offset do core de -1000 a 1000 MHz.',
    err_badMemOffset: 'Offset da memória de -3000 a 6000 MHz.',
    err_badFan: 'Modo de ventoinhas desconhecido.',
    err_badFanPct: 'Ventoinhas de 30 a 100 %.',
    err_badCurve: 'A curva precisa de 2 a 8 pontos, de 20 a 95 °C e de 30 a 100 %.',
    err_badLimit: 'Limites: core de 60 a 90 °C, memória de 70 a 100 °C.',
    helpLabel: 'Ajuda:',
    helpDiscord: 'Discord da DoriByte',
    foldMining: 'Mineração',
    untested: ' (por testar)',
    perDay: '/dia',
    earnNote: 'antes da taxa da pool e da luz',
    earnNoteKwh: 'já sem a luz, antes da taxa da pool',
    earnWait: 'a ler os preços…',
    autoTitle: 'Troca automática',
    autoHint: 'Mina a moeda que rende mais, entre os perfis marcados. Troca só quando outra rende pelo menos 10 % mais durante 15 minutos seguidos, confirmado por preços novos, e nunca antes de 15 minutos na moeda atual. Preços da CoinGecko e rede do WhatToMine, como no DoriRig.',
    autoOn: 'Ligar a troca automática',
    autoIn: 'Entra na troca',
    autoUnmeasured: 'por medir: mine 3 minutos com este perfil',
    autoNoPrice: 'sem preço ainda',
    autoKwh: 'Preço da luz (€/kWh)',
    autoKwhHint: 'Opcional: desconta a eletricidade no que cada moeda rende.',
    benchTitle: 'Qual moeda rende mais?',
    benchHint: 'O Control mina cada moeda cerca de 1 min 45 s nesta máquina e mostra quanto rende por dia. A mineração atual para e volta no fim.',
    benchAll: 'Benchmark de todas as moedas',
    benchProfile: 'Benchmark',
    benchCancel: 'Cancelar benchmark',
    benchWarm: 'a aquecer',
    benchMeasure: 'a medir',
    benchLeft: 'faltam {s} s',
    bench_waiting: 'à espera',
    bench_running: 'a correr',
    bench_done: 'feito',
    bench_failed: 'falhou',
    bench_unsupported: 'não corre nesta placa',
    bench_noWallet: 'sem carteira',
    bench_cancelled: 'cancelado',
    benchResults: 'O que cada moeda rende hoje nesta máquina',
    benchDev: 'carteira dos devs',
    benchNoOc: 'sem as definições das placas',
    benchNoPrice: 'sem preço ainda',
    benchUse: 'Minerar esta',
    benchCreate: 'Criar perfil',
    benchAgain: 'Repetir benchmark',
    benchMissing1: '{coins} não tem carteira adicionada. Adicione uma carteira {coins} ou corra o benchmark com a carteira dos devs.',
    benchMissingN: '{coins} não têm carteira adicionada. Adicione as carteiras ou corra o benchmark com a carteira dos devs.',
    benchDevNote: 'Com a carteira dos devs, o que for minerado durante o benchmark vai para os programadores do DoriMiner.',
    benchAddWallet: 'Adicionar carteira {coin}',
    benchUseDev: 'Correr com a carteira dos devs',
    benchConfirm: 'O benchmark mina {what} durante cerca de {min} min. A mineração atual para e volta no fim.',
    benchStart: 'Começar',
    benchErr_running: 'Já há um benchmark a correr.',
    benchErr_external: 'Há um miner lançado fora do Control: pare-o primeiro.',
    benchErr_nothing: 'Nenhuma moeda pode correr nesta máquina.',
    stBench: 'Benchmark',
    and: ' e ',
    cardsLeftNote: 'As placas ainda têm as definições do último perfil (potência, relógios ou ventoinhas).',
    autoStHold: 'Fica em {coin}: continua a compensar.',
    autoStAhead: '{ahead} está à frente ({net}/dia); troca daqui a ~{min} min se continuar.',
    autoStSample: '{ahead} está à frente ({net}/dia); à espera de preços novos para confirmar.',
    autoStManual: 'O perfil a minerar não está marcado: a troca automática não mexe nele.',
    autoStNoData: 'Sem preços recentes: fica como está.',
    autoStOff: 'Desligada.',
    autoStStopped: 'Liga a mineração (On) com um perfil marcado para a troca começar.',
    remoteTitle: 'Acesso remoto (telemóvel)',
    remoteHint: 'Ver a mineração, ligar e desligar e trocar de perfil a partir do telemóvel, pela app DoriRig ou pelo browser. Precisa do Tailscale (gratuito, tailscale.com) neste computador e no telemóvel, com a mesma conta: nada fica aberto na internet.',
    remoteOn: 'Ligar o acesso remoto',
    remoteOnHint: 'Só os telemóveis emparelhados aqui entram; cada um pode ser revogado.',
    remotePort: 'Porta local do acesso remoto',
    remoteHttps: 'Porta HTTPS no Tailscale',
    remoteHttpsHint: '8443 deixa a 443 livre para um Dori Rig na mesma máquina.',
    remoteServe: 'Ligar ao Tailscale',
    remoteCheck: 'Verificar',
    remotePair: 'Emparelhar um telemóvel',
    remoteNamePh: 'Nome do telemóvel',
    remoteQrHint: 'Na app DoriRig, "Ler o QR" (ou abra o endereço no browser do telemóvel). O código vale 15 minutos até ser usado e só aparece agora.',
    remoteQrDone: 'Feito',
    remoteRevoke: 'Revogar',
    remoteNever: 'nunca usado',
    remoteExpired: 'caducado',
    remoteVerified: 'aplicação verificada',
    remoteLast: 'último uso',
    remoteNoTs: 'O Tailscale não está instalado ou não está ligado neste computador. Instale-o (tailscale.com), entre com a sua conta e carregue em Verificar.',
    remoteNotServed: 'O Tailscale ainda não aponta para o Control: carregue em "Ligar ao Tailscale".',
    remoteReady: 'Pronto em',
    remoteListenFail: 'A porta local está ocupada por outro programa: escolha outra em Avançado.',
    remoteNoDevices: 'Nenhum telemóvel emparelhado.',
    err_badDeviceName: 'Dê um nome ao telemóvel (até 40 caracteres).',
    err_tooManyDevices: 'Já há 20 dispositivos: revogue um antes.',
    err_noTailscaleHost: 'Sem Tailscale ligado neste computador.',
    err_remoteOff: 'Ligue o acesso remoto primeiro.',
    err_noTailscale: 'O Tailscale não está instalado.',
    err_tailscaleRefused: 'O Tailscale recusou',
    err_badRemotePort: 'Porta de 1024 a 65535, diferente da porta da API.',
    err_badHttpsPort: 'O Tailscale só aceita 443, 8443 ou 10000.',
  },
  en: {
    tabSummary: 'Summary',
    tabProfiles: 'Profiles',
    tabSchedule: 'Schedule',
    tabSettings: 'Settings',
    hashrate: 'Hashrate',
    shares: 'Accepted shares',
    pool: 'Pool',
    uptime: 'Mining for',
    energy: 'Since start',
    energyToday2: 'Today',
    energy30: '30 days',
    income: 'Income',
    currency: 'Currency',
    machineName: 'Name of this machine',
    machineNameHint: "Shown here and on the phone. Left empty, the system's stays. It does not change the profiles' worker.",
    err_badMachineName: 'The name can have up to 40 characters, with no control characters.',
    segWallets: 'Wallets',
    segProfiles: 'Profiles',
    segPools: 'Pools',
    knownPools: 'Known pools',
    poolTested: 'Tested',
    poolUntested: 'Not tested yet',
    addPool: 'New pool',
    addPoolHint: 'Any {coin} pool: give its name and address.',
    benchTitle: 'Benchmark',
    benchNone: 'Not done yet.',
    energyMining: 'mining',
    energyToday: 'today',
    power: 'Power',
    perWatt: 'per watt',
    minerOutput: 'What the miner prints',
    profile: 'Profile',
    coin: 'Coin',
    worker: 'Name of this machine (worker)',
    wallet: 'Wallet',
    wallets: 'Wallets',
    walletHintQTC: 'Starts with "qz". Kept on this computer only.',
    walletHintPRL: 'Starts with "prl1". Kept on this computer only. Pearl is mined on RTX 30, 40 and 50 cards.',
    walletHintVELA: 'Starts with "vela1". Kept on this computer only. Vela is mined on RTX 30, 40 and 50 cards; the miner downloads the model (1.2 GB) on its first start.',
    walletHintERG: 'Starts with "9". Kept on this computer only. Ergo is mined on RTX 20, 30, 40 and 50 cards with 8 GB or more. On Lithos your own node pays.',
    usedByMiner: 'In use by another miner: {name}',
    usedByProgram: 'In use by another program: {name}',
    usedByUnknown: 'In use by another program',
    noticeErgTable: 'The Ergo table no longer fits in this card\'s memory: it keeps mining, more slowly.',
    pools: 'Pools',
    name: 'Name',
    address: 'Address',
    devFee: 'DoriMiner support (%)',
    devFeeHint: '1% of the time mines for the DoriByte wallet, on the same pool. 0 turns it off.',
    devices: 'Cards',
    devicesHint: 'Empty: all of them. Or nvidia-smi numbers, separated by commas.',
    showWindow: "Show the miner's window",
    showWindowHint: "The miner's own window, with its banner and reports. Hidden, what it prints shows in the Summary.",
    startWithControl: 'Mine when Control opens',
    startWithControlHint: 'Within the schedule, when it is on.',
    restartOnCrash: 'Start the miner again if it stops by itself',
    advanced: 'Advanced',
    minerPath: 'Where dori-miner is',
    browse: 'Browse…',
    apiPort: "Miner's API port",
    language: 'Language',
    langAuto: "The system's",
    save: 'Save',
    cancel: 'Cancel',
    edit: 'Edit',
    remove: 'Remove',
    use: 'Use',
    active: 'Active',
    saved: 'Saved.',
    savedRestarted: 'Saved. The miner started again with the new settings.',
    fixErrors: 'Some fields need fixing.',
    newProfile: 'New profile',
    newWallet: 'New wallet',
    newPool: 'New pool',
    addPool: 'Add a pool…',
    walletNew: '+ New wallet…',
    noWallet: '(no wallet)',
    poolOrder: 'In order: when the first one does not answer for 2 minutes the next is used, and the first is tried again every hour.',
    up: 'Up',
    down: 'Down',
    cardsSection: 'Cards',
    cardsHint: 'Empty = leave as is. Changing the cards needs Control opened as administrator; everything goes back when the miner stops.',
    powerW: 'Power limit (W)',
    coreLock: 'Locked core (MHz)',
    memLock: 'Locked memory (MHz)',
    coreOffset: 'Core offset (MHz)',
    memOffset: 'Memory offset (MHz)',
    fans: 'Fans',
    fanAuto: "The card's automatic",
    fanFixed: 'Fixed',
    fanCurve: 'Curve',
    fanPct: 'Percentage',
    curveHint: 'Temperature (°C) → fan (%). Looks at the core, or at the memory 20 °C lower when that is hotter.',
    addPoint: 'Add a point',
    limitsSection: 'Temperature limits',
    limitsHint: 'Near a limit the fans go to 100%; a minute over it, the miner stops and comes back when the card cools down.',
    coreLimit: 'Core (°C)',
    memLimit: 'Memory (°C)',
    walletsHint: 'Kept on this computer only.',
    poolsHint: "Kryptex and Suprnova pools are already in the profiles' list. Add others here.",
    inUse: 'in use',
    scheduleOn: 'Mine only at certain hours',
    scheduleHint: 'With the switch on, Control starts and stops the miner at these hours, and may change profile. A slot that ends before it starts runs past midnight.',
    addSlot: 'Add a slot',
    activeProfile: 'Active profile',
    from: 'from',
    to: 'to',
    days: ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'],
    stMining: 'Mining',
    stStopped: 'Stopped',
    stWaiting: 'Waiting for the schedule',
    stRestarting: 'Restarting',
    stExternal: 'Miner started outside Control',
    stCooling: 'Cooling down',
    stNeedAdmin: 'Needs administrator',
    connected: 'connected',
    disconnected: 'not connected',
    rejected: 'rejected',
    stale: 'stale',
    devFeeNow: 'mining the support now',
    miner: 'Miner',
    notFound: 'not found',
    found: 'found',
    temp: 'Core',
    memTemp: 'Memory',
    fan: 'Fan',
    clock: 'Clock',
    load: 'Load',
    rescue: 'rescue',
    noGpu: 'nvidia-smi did not answer: no card readings.',
    auto: 'automatic',
    other: 'Other…',
    pin: 'Keep on top of other windows',
    unpin: 'Stop staying on top',
    toWindow: 'Open as a big window',
    toCompact: 'Compact mode (at the side of the screen)',
    elevate: 'Open as administrator',
    withoutOc: 'Mine without changing the cards',
    restoreCards: 'Put the cards back',
    ocOk: 'Cards: profile applied',
    ocPartial: 'Cards: memory lock refused',
    ocNone: 'Cards: unchanged',
    adminYes: 'Control runs as administrator: it can change the cards.',
    adminNo: 'Control is not running as administrator: profiles that change the cards ask to open it that way.',
    err_unknownCoin: 'Unknown coin.',
    err_badWallet: 'Not an address of this coin.',
    err_badPoolUrl: 'Use [stratum+tcp:// or stratum+ssl://]host:port.',
    err_unknownPool: 'Unknown pool, or of another coin.',
    err_badPools: '1 to 5 pools.',
    err_badWorker: 'Letters, digits, - and _ (up to 32).',
    err_badDevFee: '0 to 5.',
    err_badDevices: 'Card numbers separated by commas, e.g. 0,1.',
    err_badMinerPath: 'A full path, without % or quotes.',
    err_badFlag: 'Invalid value in the settings.',
    err_badApiPort: '1024 to 65535.',
    err_badSlot: 'Pick the days and two different times.',
    err_badName: 'Give it a name (up to 40 letters).',
    err_badId: 'Repeated id.',
    err_unknownWallet: 'Unknown wallet.',
    err_walletOtherCoin: 'This wallet is for another coin.',
    err_unknownProfile: 'Unknown profile.',
    err_badPowerW: 'Power limit 50 to 1000 W.',
    err_badCoreLock: 'Locked core 300 to 4000 MHz.',
    err_badMemLock: 'Locked memory 100 to 20000 MHz.',
    err_badCoreOffset: 'Core offset -1000 to 1000 MHz.',
    err_badMemOffset: 'Memory offset -3000 to 6000 MHz.',
    err_badFan: 'Unknown fan mode.',
    err_badFanPct: 'Fans 30 to 100%.',
    err_badCurve: 'The curve needs 2 to 8 points, 20 to 95 °C and 30 to 100%.',
    err_badLimit: 'Limits: core 60 to 90 °C, memory 70 to 100 °C.',
    helpLabel: 'Help:',
    helpDiscord: 'DoriByte Discord',
    foldMining: 'Mining',
    untested: ' (not mined on yet)',
    perDay: '/day',
    earnNote: 'before the pool fee and the power',
    earnNoteKwh: 'power taken off, before the pool fee',
    earnWait: 'reading the prices…',
    autoTitle: 'Automatic switch',
    autoHint: 'Mines the coin that pays most among the profiles ticked. It switches only when another pays at least 10% more for 15 minutes in a row, confirmed by fresh prices, and never before 15 minutes on the current coin. Prices from CoinGecko and the network from WhatToMine, as in DoriRig.',
    autoOn: 'Turn the automatic switch on',
    autoIn: 'Takes part',
    autoUnmeasured: 'not measured: mine 3 minutes with this profile',
    autoNoPrice: 'no price yet',
    autoKwh: 'Power price (€/kWh)',
    autoKwhHint: 'Optional: takes the electricity off what each coin pays.',
    benchTitle: 'Which coin pays more?',
    benchHint: 'Control mines each coin for about 1 min 45 s on this computer and shows what it pays a day. Mining stops and comes back at the end.',
    benchAll: 'Benchmark every coin',
    benchProfile: 'Benchmark',
    benchCancel: 'Cancel benchmark',
    benchWarm: 'warming up',
    benchMeasure: 'measuring',
    benchLeft: '{s} s left',
    bench_waiting: 'waiting',
    bench_running: 'running',
    bench_done: 'done',
    bench_failed: 'failed',
    bench_unsupported: 'does not run on this card',
    bench_noWallet: 'no wallet',
    bench_cancelled: 'cancelled',
    benchResults: 'What each coin pays today on this computer',
    benchDev: "developers' wallet",
    benchNoOc: 'without the card settings',
    benchNoPrice: 'no price yet',
    benchUse: 'Mine this one',
    benchCreate: 'Create profile',
    benchAgain: 'Run the benchmark again',
    benchMissing1: "{coins} has no wallet. Add a {coins} wallet or run the benchmark with the developers' wallet.",
    benchMissingN: "{coins} have no wallet. Add the wallets or run the benchmark with the developers' wallet.",
    benchDevNote: "With the developers' wallet, what is mined during the benchmark goes to the DoriMiner developers.",
    benchAddWallet: 'Add a {coin} wallet',
    benchUseDev: "Run with the developers' wallet",
    benchConfirm: 'The benchmark mines {what} for about {min} min. Mining stops and comes back at the end.',
    benchStart: 'Start',
    benchErr_running: 'A benchmark is already running.',
    benchErr_external: 'A miner started outside Control is running: stop it first.',
    benchErr_nothing: 'No coin can run on this computer.',
    stBench: 'Benchmark',
    and: ' and ',
    cardsLeftNote: "The cards still hold the last profile's settings (power, clocks or fans).",
    autoStHold: 'Stays on {coin}: it still pays best.',
    autoStAhead: '{ahead} is ahead ({net}/day); switches in ~{min} min if it stays so.',
    autoStSample: '{ahead} is ahead ({net}/day); waiting for fresh prices to confirm.',
    autoStManual: 'The profile mining is not ticked: the switch leaves it alone.',
    autoStNoData: 'No recent prices: stays as it is.',
    autoStOff: 'Off.',
    autoStStopped: 'Switch mining On with a ticked profile for the switch to start.',
    remoteTitle: 'Remote access (phone)',
    remoteHint: 'See the mining, switch it on and off and change the profile from the phone, with the DoriRig app or a browser. Needs Tailscale (free, tailscale.com) on this computer and on the phone, with the same account: nothing is opened to the internet.',
    remoteOn: 'Turn remote access on',
    remoteOnHint: 'Only phones paired here get in; each one can be revoked.',
    remotePort: 'Remote access local port',
    remoteHttps: 'HTTPS port on Tailscale',
    remoteHttpsHint: '8443 leaves 443 free for a Dori Rig on the same machine.',
    remoteServe: 'Put on Tailscale',
    remoteCheck: 'Check',
    remotePair: 'Pair a phone',
    remoteNamePh: 'Phone name',
    remoteQrHint: 'In the DoriRig app, "Scan the QR" (or open the address in the phone\'s browser). The code is good for 15 minutes until used and is only shown now.',
    remoteQrDone: 'Done',
    remoteRevoke: 'Revoke',
    remoteNever: 'never used',
    remoteExpired: 'expired',
    remoteVerified: 'verified app',
    remoteLast: 'last used',
    remoteNoTs: 'Tailscale is not installed or not connected on this computer. Install it (tailscale.com), sign in and press Check.',
    remoteNotServed: 'Tailscale does not point at Control yet: press "Put on Tailscale".',
    remoteReady: 'Ready at',
    remoteListenFail: 'The local port is taken by another program: pick another one in Advanced.',
    remoteNoDevices: 'No phone paired.',
    err_badDeviceName: 'Give the phone a name (up to 40 characters).',
    err_tooManyDevices: 'There are 20 devices already: revoke one first.',
    err_noTailscaleHost: 'No Tailscale connected on this computer.',
    err_remoteOff: 'Turn remote access on first.',
    err_noTailscale: 'Tailscale is not installed.',
    err_tailscaleRefused: 'Tailscale refused',
    err_badRemotePort: 'Port 1024 to 65535, not the API port.',
    err_badHttpsPort: 'Tailscale only takes 443, 8443 or 10000.',
  },
};

let lang = 'pt';
let cfg = null;
let coins = {};
let policy = null;
let defaultCurve = [];
let slots = [];
let lastStatus = null;

let editing = null;
const $ = (id) => document.getElementById(id);
const t = (key) => T[lang][key] ?? T.en[key] ?? key;
const clone = (x) => JSON.parse(JSON.stringify(x));

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (v === null || v === undefined || v === false) continue;
    if (k === 'class') node.className = v;
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else if (k === 'value') node.value = v;
    else if (k === 'checked' || k === 'selected' || k === 'disabled') node[k] = Boolean(v);
    else node.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children.flat()) if (c !== null && c !== undefined && c !== false) node.append(c);
  return node;
}

function translate() {
  document.documentElement.lang = lang;
  for (const node of document.querySelectorAll('[data-i]')) node.textContent = t(node.dataset.i);
}

function hashrate(h, algo) {

  if (algo === 'vela') return `${(h || 0).toFixed(1)} inf/s`;
  if (!h) return '0 H/s';
  const units = ['H/s', 'kH/s', 'MH/s', 'GH/s', 'TH/s'];
  let i = 0;
  while (h >= 1000 && i < units.length - 1) {
    h /= 1000;
    i += 1;
  }
  return `${h.toFixed(h >= 100 ? 0 : 1)} ${units[i]}`;
}

function duration(s) {
  const d = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${m}m`;
  return `${m}m ${Math.floor(s % 60)}s`;
}

const LOGOS = { PRL: 'prl.png', QTC: 'qtc.png', ERG: 'erg.jpg', VELA: 'vela.svg' };
const coinIcon = (coin, size = 18) => (LOGOS[coin] ? el('img', { class: 'coin-ico', src: `coins/${LOGOS[coin]}`, alt: '', width: size, height: size }) : null);

const euros = (v) => {
  const usd = cfg?.currency === 'usd' && lastStatus?.usdPerEur;
  const text = (usd ? v * usd : v).toLocaleString(lang === 'pt' ? 'pt-PT' : 'en-GB', { minimumFractionDigits: 2, maximumFractionDigits: 2 });
  return usd ? `$${text}` : `${text} €`;
};

function remembering() {
  for (const d of document.querySelectorAll('details.fold[data-fold]')) {
    try {
      const v = localStorage.getItem(`fold:${d.dataset.fold}`);
      if (v !== null) d.open = v === '1';
    } catch {}
    d.addEventListener('toggle', () => {
      try {
        localStorage.setItem(`fold:${d.dataset.fold}`, d.open ? '1' : '0');
      } catch {}
    });
  }
}

function fold(key, title, open, ...children) {
  let isOpen = open;
  try {
    const v = localStorage.getItem(`fold:${key}`);
    if (v !== null) isOpen = v === '1';
  } catch {}
  const d = el('details', { class: 'fold card', 'data-fold': key }, el('summary', {}, title), el('div', { class: 'stack fold-body' }, ...children));
  d.open = isOpen;
  d.addEventListener('toggle', () => {
    try {
      localStorage.setItem(`fold:${key}`, d.open ? '1' : '0');
    } catch {}
  });
  return d;
}

const host = (url) => (url || '').replace(/^stratum\+(tcp|ssl):\/\//, '');
const fmt = (v, unit, digits = 0) => (v === null || v === undefined ? '–' : `${v.toFixed(digits)} ${unit}`);
const hotClass = (c, limit) => (c === null || c === undefined ? '' : c >= limit - 2 ? 'hot-bad' : c >= limit - 8 ? 'hot-warn' : '');

function poolName(ref) {
  const [kind, id] = ref.split(/:(.*)/s);
  if (kind === 'preset') {
    for (const coin of Object.values(coins)) {
      const p = coin.pools.find((x) => x.id === id);
      if (p) return p.name;
    }
  }
  if (kind === 'pool') return cfg.pools.find((p) => p.id === id)?.name ?? ref;
  return ref;
}

function newId(prefix, list) {
  const used = new Set(list.map((x) => x.id));
  for (let n = 1; ; n += 1) if (!used.has(`${prefix}${n}`)) return `${prefix}${n}`;
}

function showTab(name) {
  for (const tab of document.querySelectorAll('.tab')) tab.classList.toggle('tab-active', tab.dataset.tab === name);
  for (const n of ['summary', 'profiles', 'settings']) $(`tab-${n}`).hidden = n !== name;
  if (name === 'profiles') renderProfiles();
  if (name === 'settings') {
    refreshRemote();
    if (lastStatus) renderAuto(lastStatus);
  }
}
for (const tab of document.querySelectorAll('.tab')) tab.addEventListener('click', () => showTab(tab.dataset.tab));

const STATES = {
  mining: ['stMining', 'pill-ok'],
  stopped: ['stStopped', 'pill-muted'],
  waiting: ['stWaiting', 'pill-warn'],
  restarting: ['stRestarting', 'pill-warn'],
  external: ['stExternal', 'pill-warn'],
  cooling: ['stCooling', 'pill-bad'],
  needAdmin: ['stNeedAdmin', 'pill-warn'],
  bench: ['stBench', 'pill-warn'],
};

function renderStatus(s) {
  lastStatus = s;
  if (s.machine) document.title = `DoriMiner Control · ${s.machine}`;
  if (s.machine && $('subtitle').dataset.base) $('subtitle').textContent = `${$('subtitle').dataset.base} · ${s.machine}`;
  if (s.systemName && $('f-machine')) $('f-machine').placeholder = s.systemName;
  const [label, cls] = STATES[s.state] ?? STATES.stopped;
  for (const [id, extra] of [['state-pill', ''], ['hero-state', ' hero-state']]) {
    const pill = $(id);
    pill.textContent = t(label);
    pill.className = `pill ${cls}${extra}`;
  }

  const power = $('power');
  power.classList.toggle('power-off', !s.armed);
  $('power-text').textContent = s.armed ? 'On' : 'Off';
  power.disabled = s.state === 'external';

  const actions = [];
  if (s.state === 'needAdmin') {
    if (lastStatus.admin === false) actions.push(el('button', { class: 'btn btn-primary btn-small', type: 'button', onclick: () => window.control.elevate() }, t('elevate')));
    actions.push(el('button', { class: 'btn btn-small', type: 'button', onclick: async () => renderStatus(await window.control.mineWithoutOc()) }, t('withoutOc')));
  }

  const cardsLeft = s.cardsChanged && s.state !== 'mining' && s.state !== 'bench';
  if (cardsLeft) actions.push(el('button', { class: 'btn btn-small', type: 'button', onclick: async () => renderStatus(await window.control.restoreCards()) }, t('restoreCards')));
  $('notice').hidden = !s.note && actions.length === 0;
  $('notice-text').textContent = s.note || (cardsLeft ? t('cardsLeftNote') : '');
  $('notice-actions').replaceChildren(...actions);

  const pick = $('quick-profile');
  if (document.activeElement !== pick) {
    pick.replaceChildren(...cfg.profiles.map((p) => el('option', { value: p.id }, p.name)));
    pick.value = s.profileId ?? cfg.activeProfile;
  }
  const oc = $('oc-pill');
  const ocText = s.oc?.code === 'ok' ? ['ocOk', 'pill-ok'] : s.oc?.code === 'partial' ? ['ocPartial', 'pill-warn'] : s.state === 'mining' ? ['ocNone', 'pill-muted'] : null;
  oc.hidden = !ocText;
  if (ocText) {
    oc.textContent = t(ocText[0]);
    oc.className = `pill ${ocText[1]}`;
  }

  const sum = s.summary;

  const coin = cfg.profiles.find((p) => p.id === (s.profileId ?? cfg.activeProfile))?.coin;
  const ico = $('coin-ico');
  ico.hidden = !LOGOS[coin];
  if (LOGOS[coin] && !ico.src.endsWith(LOGOS[coin])) ico.src = `coins/${LOGOS[coin]}`;
  const earn = $('t-earn');
  if (s.earn) earn.textContent = `≈ ${euros(s.earn.net)}${t('perDay')}`;
  else earn.textContent = s.state === 'mining' ? t('earnWait') : '–';
  $('t-hash').textContent = sum ? hashrate(sum.hashrate, sum.algo) : '–';
  $('t-hash-meta').textContent = sum ? `${sum.miner} ${sum.version} · ${sum.algo}` : '';
  $('t-shares').textContent = sum ? String(sum.accepted_shares) : '–';
  $('t-shares-meta').textContent = sum ? `${sum.invalid_shares} ${t('rejected')} · ${sum.stale_shares} ${t('stale')}` : '';
  $('t-pool').textContent = sum ? (sum.pool.connected ? t('connected') : t('disconnected')) : '–';
  $('t-pool-meta').textContent = s.pool?.name ? `${s.pool.name}${s.pool.of > 1 ? ` (${s.pool.index + 1}/${s.pool.of})` : ''}` : sum ? host(sum.pool.url) : '';
  $('t-up').textContent = sum ? duration(sum.uptime) : '–';
  $('t-up-meta').textContent = sum ? (sum.dev_fee?.active ? t('devFeeNow') : `${t('devFee').replace(' (%)', '')}: ${sum.dev_fee_percent} %`) : '';

  const used = new Set((sum?.gpus ?? []).map((g) => g.id));
  const cardsUsed = s.gpus.filter((g) => used.size === 0 || used.has(g.index));
  const watts = cardsUsed.reduce((a, g) => a + (g.watts ?? 0), 0);
  $('t-watts').textContent = cardsUsed.length && watts ? `${watts.toFixed(0)} W` : '–';
  $('t-watts-meta').textContent = sum && watts ? `${hashrate(sum.hashrate / watts)} ${t('perWatt')}` : '';

  const en = s.energy;
  const kwh = (x) => `${x.kwh.toLocaleString(lang === 'pt' ? 'pt-PT' : 'en-GB', { minimumFractionDigits: 2, maximumFractionDigits: 2 })} kWh`;
  const cost = (x) => (x.eur !== null ? euros(x.eur) : '');
  $('t-energy').textContent = en ? kwh(en.session) : '–';
  $('t-energy-meta').textContent = en ? [`${duration(en.session.hours * 3600)}`, cost(en.session)].filter(Boolean).join(' · ') : '';
  $('t-energy-today').textContent = en ? kwh(en.today) : '–';
  $('t-energy-today-meta').textContent = en ? cost(en.today) : '';
  $('t-energy-30').textContent = en ? kwh(en.days30) : '–';
  $('t-energy-30-meta').textContent = en ? cost(en.days30) : '';

  renderGpus(s);
  renderBench(s);

  const showLog = s.log.length > 0 && !(s.state === 'mining' && s.showWindow);
  $('log-card').hidden = !showLog;
  if (showLog) {
    const pre = $('log');
    const atEnd = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 4;
    pre.textContent = s.log.join('\n');
    if (atEnd) pre.scrollTop = pre.scrollHeight;
  }

  if (!$('tab-settings').hidden) renderAuto(s);
  $('path-hint').textContent = `${t('miner')}: ${s.minerPath ? `${t('found')} · ${s.minerPath}` : t('notFound')}`;
  $('admin-line').textContent = s.admin ? t('adminYes') : t('adminNo');
}

function renderGpus(s) {
  const box = $('gpus');
  box.replaceChildren();
  if (s.gpus.length === 0) {
    box.append(el('p', { class: 'meta' }, t('noGpu')));
    return;
  }
  const mined = new Map((s.summary?.gpus ?? []).map((g) => [g.id, g]));
  for (const g of s.gpus) {
    const m = mined.get(g.index);
    const cell = (label, value, cls = '') => el('div', {}, el('div', { class: 'meta' }, label), el('div', { class: `v ${cls}` }, value));
    const fanText = g.fanPct === null ? t('auto') : `${fmt(g.fanPct, '%')}${s.fans?.rescue ? ` (${t('rescue')})` : ''}`;
    box.append(
      el(
        'div',
        { class: 'card' },
        el('div', { class: 'row' }, el('div', { class: 'row-main' }, el('div', { class: 'title' }, `#${g.index} ${g.name.replace(/^NVIDIA (GeForce )?/, '')}`)), m ? el('span', { class: m.failed ? 'pill pill-bad' : 'pill pill-ok' }, hashrate(m.hashrate, sum?.algo)) : null),
        el(
          'div',
          { class: 'gpu-grid' },
          cell(t('temp'), fmt(g.coreC, '°C'), hotClass(g.coreC, s.limits.coreC)),
          cell(t('memTemp'), fmt(g.memC, '°C'), hotClass(g.memC, s.limits.memC)),
          cell(t('power'), g.limitW ? `${fmt(g.watts, 'W')} / ${g.limitW.toFixed(0)}` : fmt(g.watts, 'W')),
          cell(t('fan'), fanText, s.fans?.rescue ? 'hot-bad' : ''),
          cell(t('clock'), fmt(g.clockMHz, 'MHz')),
          cell(t('load'), fmt(g.utilPct, '%')),
        ),
        ...(g.usedBy ? [el('p', { class: 'meta' }, el('span', { class: 'pill pill-warn' }, g.usedBy.miner ? t('usedByMiner').replace('{name}', g.usedBy.miner) : g.usedBy.program ? t('usedByProgram').replace('{name}', g.usedBy.program) : t('usedByUnknown')))] : []),

        ...(m?.notices ?? []).filter((c) => c === 'erg_compact_table').map(() => el('p', { class: 'meta' }, el('span', { class: 'pill pill-warn' }, 'Ergo'), ' ', t('noticeErgTable'))),
      ),
    );
  }
}

function renderAuto(s) {
  const box = $('auto-box');
  if (box.contains(document.activeElement) && document.activeElement.type === 'number') return;
  const a = s.auto || { enabled: false, profiles: [], decision: null };
  const setAuto = (patch) => save({ ...cfg, auto: { ...cfg.auto, ...patch } }, null).then(() => lastStatus && renderAuto(lastStatus));
  const line = () => {
    if (!cfg.auto.enabled) return t('autoStOff');
    const d = a.decision;
    if (!d) return t('autoStStopped');
    if (d.kind === 'manual') return t('autoStManual');
    if (d.kind === 'no-data') return t('autoStNoData');
    if (d.ahead) return t(d.ahead.needsSample && d.ahead.waitMin === 0 ? 'autoStSample' : 'autoStAhead').replace('{ahead}', d.ahead.coin).replace('{net}', euros(d.ahead.net)).replace('{min}', String(d.ahead.waitMin));
    return t('autoStHold').replace('{coin}', d.coin);
  };
  box.replaceChildren(
    el('p', { class: 'hint' }, t('autoHint')),
    el('label', { class: 'check' }, el('input', { type: 'checkbox', checked: cfg.auto.enabled, onchange: (e) => setAuto({ enabled: e.target.checked }) }), el('span', {}, el('span', { class: 'title' }, t('autoOn')))),
    el('p', { class: 'auto-status' }, line()),
    el(
      'div',
      { class: 'auto-list' },
      el('div', { class: 'meta' }, t('autoIn')),
      a.profiles.map((p) =>
        el(
          'label',
          { class: 'auto-row' },
          el('input', { type: 'checkbox', checked: p.inSwitch, onchange: (e) => setAuto({ profiles: e.target.checked ? [...cfg.auto.profiles, p.id] : cfg.auto.profiles.filter((x) => x !== p.id) }) }),
          coinIcon(p.coin),
          el('span', { class: 'auto-name' }, p.name),
          el('span', { class: p.net !== null ? 'auto-val' : 'auto-val meta' }, p.net !== null ? `≈ ${euros(p.net)}${t('perDay')}` : p.measured ? t('autoNoPrice') : t('autoUnmeasured')),
        ),
      ),
    ),
    field(
      t('autoKwh'),
      el('input', { type: 'number', min: '0', max: '2', step: '0.01', value: cfg.auto.kwhEur ?? '', onchange: (e) => setAuto({ kwhEur: e.target.value === '' ? null : Number(e.target.value) }) }),
      null,
      t('autoKwhHint'),
    ),
  );
}

$('power').addEventListener('click', async () => {
  const on = $('power').classList.contains('power-off');
  $('power').disabled = true;
  renderStatus(on ? await window.control.start() : await window.control.stop());
  $('power').disabled = false;
});

$('quick-profile').addEventListener('change', async () => {
  await save({ ...cfg, activeProfile: $('quick-profile').value }, null);
});

function dialog(text, note, buttons) {
  const back = $('modal');
  const close = () => {
    back.hidden = true;
    back.replaceChildren();
  };
  back.replaceChildren(
    el(
      'div',
      { class: 'modal card stack', role: 'dialog', 'aria-modal': 'true' },
      el('p', { class: 'modal-text' }, text),
      note ? el('p', { class: 'hint' }, note) : null,
      el(
        'div',
        { class: 'actions' },
        buttons.map(([label, primary, act]) =>
          el('button', { class: `btn btn-small${primary ? ' btn-primary' : ''}`, type: 'button', onclick: () => { close(); act?.(); } }, label),
        ),
      ),
    ),
  );
  back.hidden = false;
  back.querySelector('.btn-primary, .btn')?.focus();
}

const coinList = (list) => (list.length < 2 ? list.join('') : `${list.slice(0, -1).join(', ')}${t('and')}${list[list.length - 1]}`);

async function askBench(scope) {
  const plan = await window.control.benchPlan(scope);
  const go = async (useDev) => {
    const r = await window.control.benchStart(scope, useDev);
    if (!r.ok) dialog(t(`benchErr_${r.code}`), null, [[t('cancel'), true, null]]);
    else {
      renderStatus(r.status);
      showTab('profiles');
    }
  };
  const runnable = plan.items.filter((x) => x.supported);
  if (runnable.length === 0) return dialog(t('benchErr_nothing'), null, [[t('cancel'), true, null]]);
  const what = scope.kind === 'profile' ? `"${plan.items[0]?.name}"` : coinList(runnable.map((x) => x.coin));
  const minutes = (sec) => String(Math.max(1, Math.round(sec / 60)));
  if (plan.missing.length > 0) {
    const text = t(plan.missing.length === 1 ? 'benchMissing1' : 'benchMissingN').replaceAll('{coins}', coinList(plan.missing));
    const note = `${t('benchDevNote')} ${t('benchConfirm').replace('{what}', what).replace('{min}', minutes(plan.seconds.dev))}`;
    return dialog(text, note, [
      ...plan.missing.map((coin) => [t('benchAddWallet').replace('{coin}', coin), false, () => { showTab('profiles'); editWallet(-1, coin); }]),
      [t('benchUseDev'), true, () => go(true)],
      [t('cancel'), false, null],
    ]);
  }
  dialog(t('benchConfirm').replace('{what}', what).replace('{min}', minutes(plan.seconds.own)), null, [
    [t('benchStart'), true, () => go(false)],
    [t('cancel'), false, null],
  ]);
}

function renderBench(s) {
  const b = s.bench;
  const box = $('bench-card');
  if (!b) {
    box.hidden = true;
    return;
  }
  box.hidden = false;
  const head = el('div', { class: 'section-head' }, el('h2', {}, t('benchTitle')));
  if (b.active) {
    head.append(el('button', { class: 'btn btn-small', type: 'button', onclick: async () => renderStatus(await window.control.benchCancel()) }, t('benchCancel')));
    box.replaceChildren(
      head,
      el(
        'div',
        { class: 'stack bench-list' },
        b.items.map((x) =>
          el(
            'div',
            { class: 'bench-row' },
            coinIcon(x.coin),
            el('span', { class: 'bench-name' }, x.name),
            el('span', { class: 'meta' }, x.state === 'running' ? `${t(b.phase === 'warm' ? 'benchWarm' : 'benchMeasure')} · ${t('benchLeft').replace('{s}', String(b.left ?? 0))}` : t(`bench_${x.state}`)),
            el('span', { class: 'bench-val' }, x.hps ? hashrate(x.hps, coins[x.coin]?.miner) : ''),
          ),
        ),
      ),
    );
    return;
  }
  const rows = b.results.map((r) => {
    const profile = r.profileId ? cfg.profiles.find((p) => p.id === r.profileId) : cfg.profiles.find((p) => p.coin === r.coin);
    const active = profile && profile.id === (s.profileId ?? cfg.activeProfile) && s.state === 'mining';
    const tags = [r.dev ? t('benchDev') : null, r.noOc ? t('benchNoOc') : null].filter(Boolean).join(' · ');
    return el(
      'div',
      { class: 'bench-row' },
      coinIcon(r.coin),
      el('span', { class: 'bench-name' }, coins[r.coin]?.short ?? r.coin, tags ? el('span', { class: 'meta' }, ` · ${tags}`) : null),
      el('span', { class: 'meta' }, `${hashrate(r.hps, coins[r.coin]?.miner)}${r.watts ? ` · ${r.watts.toFixed(0)} W` : ''}`),
      el('span', { class: r.net !== null ? 'bench-val' : 'bench-val meta' }, r.net !== null ? `≈ ${euros(r.net)}${t('perDay')}` : t('benchNoPrice')),
      active
        ? el('span', { class: 'pill pill-ok' }, t('inUse'))
        : profile
          ? el('button', { class: 'btn btn-small', type: 'button', onclick: async () => { await save({ ...cfg, activeProfile: profile.id }, null); renderStatus(await window.control.start()); } }, t('benchUse'))
          : el('button', { class: 'btn btn-small', type: 'button', onclick: () => { showTab('profiles'); editProfile(-1, r.coin); } }, t('benchCreate')),
    );
  });
  const unsupported = b.unsupported
    .filter((c) => !b.results.some((r) => r.coin === c))
    .map((c) => el('div', { class: 'bench-row' }, coinIcon(c), el('span', { class: 'bench-name' }, coins[c]?.short ?? c), el('span', { class: 'meta' }, t('bench_unsupported'))));
  const failed = b.items.filter((x) => x.state === 'failed').map((x) => el('p', { class: 'error' }, `${x.coin}: ${x.reason ?? t('bench_failed')}`));
  box.replaceChildren(...[
    head,
    rows.length ? el('p', { class: 'meta' }, t('benchResults')) : el('p', { class: 'hint' }, t('benchHint')),
    rows.length ? el('div', { class: 'stack bench-list' }, rows, unsupported) : null,
    ...failed,
    rows.length ? el('p', { class: 'hint' }, t(cfg.auto.kwhEur ? 'earnNoteKwh' : 'earnNote')) : null,
    el(
      'div',
      { class: 'actions' },
      el('button', { class: `btn btn-small${rows.length ? '' : ' btn-primary'}`, type: 'button', disabled: s.state === 'external', onclick: () => askBench({ kind: 'all' }) }, t(rows.length ? 'benchAgain' : 'benchAll')),
    ),
  ].filter(Boolean));
}

let viewState = { mode: 'compact', alwaysOnTop: false };

function renderView() {
  document.body.classList.toggle('compact', viewState.mode === 'compact');
  const pin = $('pin');
  pin.setAttribute('aria-pressed', String(viewState.alwaysOnTop));
  pin.title = t(viewState.alwaysOnTop ? 'unpin' : 'pin');
  pin.setAttribute('aria-label', pin.title);
  const mode = $('mode');
  mode.title = t(viewState.mode === 'compact' ? 'toWindow' : 'toCompact');
  mode.setAttribute('aria-label', mode.title);
}

$('mode').addEventListener('click', async () => {
  viewState = await window.control.view({ mode: viewState.mode === 'compact' ? 'window' : 'compact' });
  renderView();
});
$('pin').addEventListener('click', async () => {
  viewState = await window.control.view({ alwaysOnTop: !viewState.alwaysOnTop });
  renderView();
});

async function save(next, msgNode) {
  const r = await window.control.save(next);
  if (!r.ok) {
    if (msgNode) msgNode.textContent = t('fixErrors');
    return r.errors;
  }
  cfg = r.config;
  if (r.lang !== lang) {
    lang = r.lang;
    translate();
    renderView();
  }
  fillSettings();
  slots = clone(cfg.schedule.slots);
  renderSlots();
  if (msgNode) msgNode.textContent = r.restarted ? t('savedRestarted') : t('saved');
  return null;
}

const errText = (errors, key) => (errors && errors[key] ? t(`err_${errors[key]}`) : '');

function field(label, input, error, hint) {
  return el('label', { class: 'field' }, el('span', { class: 'label' }, label), input, hint ? el('span', { class: 'hint' }, hint) : null, error ? el('span', { class: 'error' }, error) : null);
}

function numInput(value, onchange, attrs = {}) {
  const input = el('input', { type: 'number', value: value ?? '', ...attrs });
  input.addEventListener('input', () => onchange(input.value === '' ? null : Number(input.value)));
  return input;
}

function textInput(value, onchange, attrs = {}) {
  const input = el('input', { type: 'text', value: value ?? '', spellcheck: 'false', ...attrs });
  input.addEventListener('input', () => onchange(input.value));
  return input;
}

function select(options, value, onchange) {
  const s = el('select', {}, options.map(([v, label]) => el('option', { value: v, selected: v === value }, label)));
  s.addEventListener('change', () => onchange(s.value));
  return s;
}

let selectedCoin = null;
let segment = 'profiles';

function renderProfiles() {
  const box = $('profiles-view');
  if (editing?.kind === 'profile') return box.replaceChildren(profileEditor());
  if (editing?.kind === 'wallet') return box.replaceChildren(walletEditor());
  if (editing?.kind === 'pool') return box.replaceChildren(poolEditor());
  const inForce = lastStatus?.profileId ?? cfg.activeProfile;
  if (!coins[selectedCoin]) selectedCoin = cfg.profiles.find((p) => p.id === inForce)?.coin ?? Object.keys(coins)[0];

  const profiles = el(
    'div',
    { class: 'card stack' },
    el('div', { class: 'section-head' }, el('h2', {}, t('tabProfiles')), el('button', { class: 'manage-only btn btn-small btn-primary', type: 'button', onclick: () => editProfile(-1, selectedCoin) }, t('newProfile'))),
    cfg.profiles.map((p, i) => {
      if (p.coin !== selectedCoin) return null;
      const w = cfg.wallets.find((x) => x.id === p.walletId);
      const oc = [p.oc.powerW != null ? `${p.oc.powerW} W` : null, p.oc.coreLockMHz != null ? `core ${p.oc.coreLockMHz}` : null, p.oc.fan.mode !== 'auto' ? `${t('fans').toLowerCase()} ${p.oc.fan.mode === 'fixed' ? `${p.oc.fan.pct} %` : t('fanCurve').toLowerCase()}` : null].filter(Boolean).join(' · ');
      return el(
        'div',
        { class: 'list-item' },
        el(
          'div',
          { class: 'row-main' },
          el('div', { class: 'title' }, coinIcon(p.coin), p.name, ' ', p.id === cfg.activeProfile ? el('span', { class: 'pill pill-ok' }, t('active')) : null, p.id === inForce && lastStatus?.state === 'mining' ? el('span', { class: 'pill pill-muted' }, t('inUse')) : null),
          el('div', { class: 'meta' }, [coins[p.coin]?.short ?? p.coin, w ? w.name : t('noWallet'), p.pools.map(poolName).join(' → ')].join(' · ')),
          oc ? el('div', { class: 'meta' }, oc) : null,
        ),
        el(
          'div',
          { class: 'actions' },
          p.id !== cfg.activeProfile ? el('button', { class: 'btn btn-small', type: 'button', onclick: () => save({ ...cfg, activeProfile: p.id }, null).then(renderProfiles) }, t('use')) : null,
          el('button', { class: 'manage-only btn btn-small', type: 'button', disabled: lastStatus?.bench?.active, onclick: () => askBench({ kind: 'profile', id: p.id }) }, t('benchProfile')),
          el('button', { class: 'manage-only btn btn-small', type: 'button', onclick: () => editProfile(i) }, t('edit')),
          cfg.profiles.length > 1
            ? el(
                'button',
                {
                  class: 'btn btn-small btn-danger',
                  type: 'button',
                  onclick: () => {
                    const next = clone(cfg);
                    next.profiles.splice(i, 1);
                    if (next.activeProfile === p.id) next.activeProfile = next.profiles[0].id;
                    next.schedule.slots = next.schedule.slots.map((s) => (s.profileId === p.id ? { ...s, profileId: null } : s));
                    save(next, null).then(renderProfiles);
                  },
                },
                t('remove'),
              )
            : null,
        ),
      );
    }),
  );

  const usedWallet = (id) => cfg.profiles.some((p) => p.walletId === id);
  const wallets = el(
    'div',
    { class: 'card stack' },
    el('div', { class: 'section-head' }, el('p', { class: 'hint' }, t('walletsHint')), el('button', { class: 'manage-only btn btn-small', type: 'button', onclick: () => editWallet(-1, selectedCoin) }, t('newWallet'))),
    cfg.wallets.map((w, i) =>
      w.coin !== selectedCoin ? null : el(
        'div',
        { class: 'list-item' },
        el('div', { class: 'row-main' }, el('div', { class: 'title' }, coinIcon(w.coin), w.name), el('div', { class: 'meta mono-cut' }, `${coins[w.coin]?.short ?? w.coin} · ${w.address.slice(0, 10)}…${w.address.slice(-6)}`)),
        el(
          'div',
          { class: 'actions' },
          el('button', { class: 'manage-only btn btn-small', type: 'button', onclick: () => editWallet(i) }, t('edit')),
          !usedWallet(w.id)
            ? el(
                'button',
                {
                  class: 'btn btn-small btn-danger',
                  type: 'button',
                  onclick: () => {
                    const next = clone(cfg);
                    next.wallets.splice(i, 1);
                    save(next, null).then(renderProfiles);
                  },
                },
                t('remove'),
              )
            : null,
        ),
      ),
    ),
  );

  const usedPool = (id) => cfg.profiles.some((p) => p.pools.includes(`pool:${id}`));
  const pools = el(
    'div',
    { class: 'card stack' },
    el('div', { class: 'section-head' }, el('p', { class: 'hint' }, t('poolsHint')), el('button', { class: 'manage-only btn btn-small', type: 'button', onclick: () => editPool(-1, selectedCoin) }, t('newPool'))),
    cfg.pools.map((p, i) =>
      p.coin !== selectedCoin ? null : el(
        'div',
        { class: 'list-item' },
        el('div', { class: 'row-main' }, el('div', { class: 'title' }, coinIcon(p.coin), p.name), el('div', { class: 'meta' }, `${coins[p.coin]?.short ?? p.coin} · ${host(p.url)}`)),
        el(
          'div',
          { class: 'actions' },
          el('button', { class: 'manage-only btn btn-small', type: 'button', onclick: () => editPool(i) }, t('edit')),
          !usedPool(p.id)
            ? el(
                'button',
                {
                  class: 'btn btn-small btn-danger',
                  type: 'button',
                  onclick: () => {
                    const next = clone(cfg);
                    next.pools.splice(i, 1);
                    save(next, null).then(renderProfiles);
                  },
                },
                t('remove'),
              )
            : null,
        ),
      ),
    ),
  );

  const known = el('div', { class: 'card stack' }, el('h2', {}, t('knownPools')), (coins[selectedCoin]?.pools ?? []).map((p) => el('div', { class: 'list-item' }, el('div', { class: 'row-main' }, el('div', { class: 'title' }, p.name, ' ', el('span', { class: p.tested === false ? 'pill pill-muted' : 'pill pill-ok' }, t(p.tested === false ? 'poolUntested' : 'poolTested'))), el('div', { class: 'meta mono-cut' }, p.url)))));
  const parts = { profiles: [profiles], wallets: [wallets], pools: [pools, known] }[segment];
  drawProfilesHead();
  $('bench-card').classList.toggle('seg-hidden', segment !== 'profiles');
  box.replaceChildren(...parts);
}

function drawProfilesHead() {
  const count = (coin) => cfg.profiles.filter((p) => p.coin === coin).length;
  $('profiles-head').replaceChildren(
    el('div', { class: 'coin-bar' }, Object.keys(coins).map((coin) => el('button', { class: coin === selectedCoin ? 'chip chip-on' : 'chip', type: 'button', onclick: () => { selectedCoin = coin; editing = null; renderProfiles(); } }, coinIcon(coin), coins[coin].short, count(coin) ? el('span', { class: 'chip-count' }, String(count(coin))) : null))),
    el('div', { class: 'seg' }, [['profiles', 'segProfiles'], ['wallets', 'segWallets'], ['pools', 'segPools']].map(([id, label]) => el('button', { class: id === segment ? 'seg-on' : '', type: 'button', onclick: () => { segment = id; editing = null; renderProfiles(); } }, t(label)))),
  );
}

function editProfile(i, coinWanted = null) {
  const draft = clone(cfg);
  if (i < 0) {
    const coin = coins[coinWanted] ? coinWanted : Object.keys(coins)[0];
    draft.profiles.push({
      id: newId('p', draft.profiles),
      name: `${coins[coin].short} ${draft.profiles.length + 1}`,
      coin,
      walletId: draft.wallets.find((w) => w.coin === coin)?.id ?? null,
      pools: [`preset:${coins[coin].defaultPool}`],
      worker: draft.profiles[0]?.worker ?? 'rig',
      oc: { powerW: null, coreLockMHz: null, memLockMHz: null, coreOffsetMHz: null, memOffsetMHz: null, fan: { mode: 'auto', pct: 70, curve: null }, coreLimitC: null, memLimitC: null },
    });
    i = draft.profiles.length - 1;
  }
  editing = { kind: 'profile', index: i, draft, errors: null, newWallet: null };
  renderProfiles();
}

function profileEditor() {
  const { draft, index: i, errors } = editing;
  const p = draft.profiles[i];
  const coin = coins[p.coin];
  const err = (k) => errText(errors, `profiles.${i}.${k}`);
  const rerender = () => renderProfiles();

  const walletOptions = [['', t('noWallet')], ...draft.wallets.filter((w) => w.coin === p.coin).map((w) => [w.id, w.name]), ['__new', t('walletNew')]];
  const walletSelect = select(walletOptions, editing.newWallet ? '__new' : p.walletId ?? '', (v) => {
    if (v === '__new') editing.newWallet = { name: `${coin.short} ${draft.wallets.length + 1}`, address: '' };
    else {
      editing.newWallet = null;
      p.walletId = v || null;
    }
    rerender();
  });
  const newWallet = editing.newWallet
    ? el(
        'div',
        { class: 'group stack' },
        field(t('name'), textInput(editing.newWallet.name, (v) => (editing.newWallet.name = v), { maxlength: 40 })),
        field(t('address'), textInput(editing.newWallet.address, (v) => (editing.newWallet.address = v), { class: 'mono' }), errText(errors, `wallets.${draft.wallets.length}.address`), T[lang][`walletHint${p.coin}`] ?? coin.walletHint),
      )
    : null;

  const allPools = [...coin.pools.map((x) => [`preset:${x.id}`, x.name + (x.tested === false ? t('untested') : '')]), ...draft.pools.filter((x) => x.coin === p.coin).map((x) => [`pool:${x.id}`, x.name])];
  const poolRows = p.pools.map((ref, k) =>
    el(
      'div',
      { class: 'pool-row' },
      el('span', { class: 'pool-n' }, String(k + 1)),
      el('span', { class: 'pool-name' }, poolName(ref)),
      el('button', { class: 'icon-btn small', type: 'button', title: t('up'), disabled: k === 0, onclick: () => { [p.pools[k - 1], p.pools[k]] = [p.pools[k], p.pools[k - 1]]; rerender(); } }, '↑'),
      el('button', { class: 'icon-btn small', type: 'button', title: t('down'), disabled: k === p.pools.length - 1, onclick: () => { [p.pools[k + 1], p.pools[k]] = [p.pools[k], p.pools[k + 1]]; rerender(); } }, '↓'),
      el('button', { class: 'icon-btn small', type: 'button', title: t('remove'), disabled: p.pools.length === 1, onclick: () => { p.pools.splice(k, 1); rerender(); } }, '×'),
    ),
  );
  const addPool = select([['', t('addPool')], ...allPools.filter(([ref]) => !p.pools.includes(ref))], '', (v) => {
    if (v && p.pools.length < 5) p.pools.push(v);
    rerender();
  });

  const oc = p.oc;
  const fan = oc.fan;
  const fanBox = el('div', { class: 'stack' }, select([['auto', t('fanAuto')], ['fixed', t('fanFixed')], ['curve', t('fanCurve')]], fan.mode, (v) => {
    fan.mode = v;
    if (v === 'curve' && !Array.isArray(fan.curve)) fan.curve = clone(defaultCurve);
    rerender();
  }));
  if (fan.mode === 'fixed') fanBox.append(field(t('fanPct'), numInput(fan.pct, (v) => (fan.pct = v), { min: 30, max: 100 })));
  if (fan.mode === 'curve') {
    fanBox.append(el('p', { class: 'hint' }, t('curveHint')));
    fan.curve.forEach((pt, k) => {
      fanBox.append(
        el(
          'div',
          { class: 'curve-row' },
          numInput(pt.tempC, (v) => (pt.tempC = v), { min: 20, max: 95 }),
          el('span', { class: 'meta' }, '°C →'),
          numInput(pt.pct, (v) => (pt.pct = v), { min: 30, max: 100 }),
          el('span', { class: 'meta' }, '%'),
          el('button', { class: 'icon-btn small', type: 'button', title: t('remove'), disabled: fan.curve.length <= 2, onclick: () => { fan.curve.splice(k, 1); rerender(); } }, '×'),
        ),
      );
    });
    if (fan.curve.length < 8) fanBox.append(el('button', { class: 'btn btn-small', type: 'button', onclick: () => { const last = fan.curve[fan.curve.length - 1]; fan.curve.push({ tempC: Math.min(95, (last?.tempC ?? 70) + 5), pct: 100 }); rerender(); } }, t('addPoint')));
  }

  const msg = el('span', { class: 'meta' });
  return el(
    'div',
    { class: 'card stack' },
    el('h2', {}, p.name || t('newProfile')),
    el('div', { class: 'two' }, field(t('name'), textInput(p.name, (v) => (p.name = v), { maxlength: 40 }), err('name')), field(t('coin'), select(Object.values(coins).map((c) => [c.id, c.name]), p.coin, (v) => {
      if (!coins[v]) return;
      p.coin = v;
      p.walletId = draft.wallets.find((w) => w.coin === v)?.id ?? null;
      p.pools = [`preset:${coins[v].defaultPool}`];
      rerender();
    }), err('coin'))),
    field(t('wallet'), walletSelect, err('wallet')),
    newWallet,
    el('div', { class: 'field' }, el('span', { class: 'label' }, t('pools')), el('div', { class: 'stack pool-list' }, poolRows), addPool, el('span', { class: 'hint' }, t('poolOrder')), err('pools') ? el('span', { class: 'error' }, err('pools')) : null),
    field(t('worker'), textInput(p.worker, (v) => (p.worker = v), { maxlength: 32 }), err('worker')),
    el(
      'details',
      { class: 'computer-only', open: [oc.powerW, oc.coreLockMHz, oc.memLockMHz, oc.coreOffsetMHz, oc.memOffsetMHz].some((v) => v != null) || fan.mode !== 'auto' || oc.coreLimitC != null || oc.memLimitC != null ? true : null },
      el('summary', { class: 'label' }, t('cardsSection')),
      el(
        'div',
        { class: 'advanced-body stack' },
        el('p', { class: 'hint' }, t('cardsHint')),
        el(
          'div',
          { class: 'grid-2' },
          field(t('powerW'), numInput(oc.powerW, (v) => (oc.powerW = v), { min: 50, max: 1000 })),
          field(t('coreLock'), numInput(oc.coreLockMHz, (v) => (oc.coreLockMHz = v), { min: 300, max: 4000, step: 15 })),
          field(t('memLock'), numInput(oc.memLockMHz, (v) => (oc.memLockMHz = v), { min: 100, max: 20000 })),
          field(t('coreOffset'), numInput(oc.coreOffsetMHz, (v) => (oc.coreOffsetMHz = v), { min: -1000, max: 1000, step: 15 })),
          field(t('memOffset'), numInput(oc.memOffsetMHz, (v) => (oc.memOffsetMHz = v), { min: -3000, max: 6000, step: 50 })),
        ),
        field(t('fans'), fanBox),
        el('span', { class: 'label' }, t('limitsSection')),
        el('p', { class: 'hint' }, t('limitsHint')),
        el(
          'div',
          { class: 'grid-2' },
          field(t('coreLimit'), numInput(oc.coreLimitC, (v) => (oc.coreLimitC = v), { min: 60, max: 90, placeholder: String(policy.coreLimitC) })),
          field(t('memLimit'), numInput(oc.memLimitC, (v) => (oc.memLimitC = v), { min: 70, max: 100, placeholder: String(policy.memoryLimitC) })),
        ),
        err('oc') ? el('span', { class: 'error' }, err('oc')) : null,
      ),
    ),
    el(
      'div',
      { class: 'save-bar' },
      el(
        'button',
        {
          class: 'btn btn-primary',
          type: 'button',
          onclick: async () => {
            const next = clone(draft);
            if (editing.newWallet) {
              const id = newId('w', next.wallets);
              next.wallets.push({ id, coin: p.coin, name: editing.newWallet.name.trim(), address: editing.newWallet.address.trim() });
              next.profiles[i].walletId = id;
            }
            const errors2 = await save(next, msg);
            if (errors2) {

              if (editing) {
                editing.errors = errors2;
                rerender();
              }
            } else {
              editing = null;
              renderProfiles();
            }
          },
        },
        t('save'),
      ),
      el('button', { class: 'btn', type: 'button', onclick: () => { editing = null; renderProfiles(); } }, t('cancel')),
      msg,
    ),
  );
}

function editWallet(i, coinWanted = null) {
  const draft = clone(cfg);
  if (i < 0) {
    const coin = coins[coinWanted] ? coinWanted : Object.keys(coins)[0];
    draft.wallets.push({ id: newId('w', draft.wallets), coin, name: `${coins[coin].short} ${draft.wallets.length + 1}`, address: '' });
    i = draft.wallets.length - 1;
  }
  editing = { kind: 'wallet', index: i, draft, errors: null };
  renderProfiles();
}

function walletEditor() {
  const { draft, index: i, errors } = editing;
  const w = draft.wallets[i];
  const err = (k) => errText(errors, `wallets.${i}.${k}`);
  const inUse = cfg.profiles.some((p) => p.walletId === w.id);
  const msg = el('span', { class: 'meta' });
  return el(
    'div',
    { class: 'card stack' },
    el('h2', {}, w.name || t('newWallet')),
    field(t('name'), textInput(w.name, (v) => (w.name = v), { maxlength: 40 }), err('name')),
    field(t('coin'), select(Object.values(coins).map((c) => [c.id, c.name]), w.coin, (v) => (w.coin = v)), err('coin')),
    field(t('address'), textInput(w.address, (v) => (w.address = v), { class: 'mono' }), err('address'), T[lang][`walletHint${w.coin}`] ?? coins[w.coin]?.walletHint),
    inUse ? el('p', { class: 'hint' }, `${t('inUse')}: ${cfg.profiles.filter((p) => p.walletId === w.id).map((p) => p.name).join(', ')}`) : null,
    editorButtons(msg),
  );
}

function editPool(i, coinWanted = null) {
  const draft = clone(cfg);
  if (i < 0) {
    const coin = coins[coinWanted] ? coinWanted : Object.keys(coins)[0];
    draft.pools.push({ id: newId('pool', draft.pools), coin, name: '', url: '' });
    i = draft.pools.length - 1;
  }
  editing = { kind: 'pool', index: i, draft, errors: null };
  renderProfiles();
}

function poolEditor() {
  const { draft, index: i, errors } = editing;
  const p = draft.pools[i];
  const err = (k) => errText(errors, `pools.${i}.${k}`);
  const msg = el('span', { class: 'meta' });
  return el(
    'div',
    { class: 'card stack' },
    el('h2', {}, p.name || t('newPool')),
    field(t('name'), textInput(p.name, (v) => (p.name = v), { maxlength: 40 }), err('name')),
    field(t('coin'), select(Object.values(coins).map((c) => [c.id, c.name]), p.coin, (v) => (p.coin = v)), err('coin')),
    field(t('address'), textInput(p.url, (v) => (p.url = v), { class: 'mono', placeholder: 'stratum+ssl://host:port' }), err('url')),
    el('p', { class: 'hint' }, t('addPoolHint').replace('{coin}', coins[p.coin]?.short ?? p.coin)),
    editorButtons(msg),
  );
}

function editorButtons(msg) {
  return el(
    'div',
    { class: 'save-bar' },
    el(
      'button',
      {
        class: 'btn btn-primary',
        type: 'button',
        onclick: async () => {
          const errors = await save(editing.draft, msg);
          if (errors) {
            if (editing) editing.errors = errors;
            renderProfiles();
          } else {
            editing = null;
            renderProfiles();
          }
        },
      },
      t('save'),
    ),
    el('button', { class: 'btn', type: 'button', onclick: () => { editing = null; renderProfiles(); } }, t('cancel')),
    msg,
  );
}

function fillSettings() {
  $('f-devfee').value = cfg.devFee;
  $('f-devices').value = cfg.devices;
  $('f-window').checked = cfg.showMinerWindow;
  $('f-autostart').checked = cfg.startWithControl;
  $('f-restart').checked = cfg.restartOnCrash;
  $('f-path').value = cfg.minerPath;
  $('f-port').value = cfg.apiPort;
  $('f-lang').value = cfg.language;
  $('f-currency').value = cfg.currency ?? 'eur';
  $('f-machine').value = cfg.machineName ?? '';
  $('f-rport').value = cfg.remote.port;
  $('f-rhttps').value = String(cfg.remote.httpsPort);
  $('r-enabled').checked = cfg.remote.enabled;
  $('r-body').hidden = !cfg.remote.enabled;
}

$('browse').addEventListener('click', async () => {
  const file = await window.control.browseMiner();
  if (file) $('f-path').value = file;
});

$('settings').addEventListener('submit', async (e) => {
  e.preventDefault();
  const errors = await save(
    {
      ...cfg,
      devFee: Number($('f-devfee').value),
      devices: $('f-devices').value.replace(/\s/g, ''),
      showMinerWindow: $('f-window').checked,
      startWithControl: $('f-autostart').checked,
      restartOnCrash: $('f-restart').checked,
      minerPath: $('f-path').value.trim(),
      apiPort: Number($('f-port').value),
      language: $('f-lang').value,
      currency: $('f-currency').value,
      machineName: $('f-machine').value.trim(),
      remote: { ...cfg.remote, port: Number($('f-rport').value), httpsPort: Number($('f-rhttps').value) },
    },
    $('save-msg'),
  );
  for (const node of document.querySelectorAll('#settings [data-err]')) node.textContent = errText(errors, node.dataset.err);
  if (!errors) refreshRemote();
});

function drawQr(qr) {
  const NS = 'http://www.w3.org/2000/svg';
  const q = 2;
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('viewBox', `0 0 ${qr.size + 2 * q} ${qr.size + 2 * q}`);
  svg.setAttribute('width', '220');
  svg.setAttribute('height', '220');
  svg.setAttribute('shape-rendering', 'crispEdges');
  const bg = document.createElementNS(NS, 'rect');
  bg.setAttribute('width', '100%');
  bg.setAttribute('height', '100%');
  bg.setAttribute('fill', '#ffffff');
  svg.append(bg);
  let d = '';
  for (let y = 0; y < qr.size; y += 1) for (let x = 0; x < qr.size; x += 1) if (qr.bits[y * qr.size + x] === '1') d += `M${x + q} ${y + q}h1v1h-1z`;
  const p = document.createElementNS(NS, 'path');
  p.setAttribute('d', d);
  p.setAttribute('fill', '#000000');
  svg.append(p);
  return svg;
}

function renderRemote(info) {
  if (!info) return;
  let line;
  if (info.error) line = t('remoteListenFail');
  else if (!info.host) line = t('remoteNoTs');
  else if (!info.serving) line = t('remoteNotServed');
  else line = `${t('remoteReady')} ${info.address}`;
  $('r-status').textContent = line;
  $('r-serve').hidden = !info.host || info.serving || Boolean(info.error);
  $('r-command').hidden = !info.host || info.serving;
  $('r-command').textContent = info.command;
  $('r-pair').disabled = !info.host || !info.listening;
  const when = (iso) => new Date(iso).toLocaleString(lang);
  $('r-devices').replaceChildren(
    ...(info.devices.length === 0
      ? [el('p', { class: 'meta' }, t('remoteNoDevices'))]
      : info.devices.map((d) =>
          el(
            'div',
            { class: 'device-row' },
            el('div', {}, el('div', { class: 'title' }, d.name), el('div', { class: 'meta' }, [d.attested ? t('remoteVerified') : null, d.expired ? t('remoteExpired') : d.lastUsedAt ? `${t('remoteLast')} ${when(d.lastUsedAt)}` : t('remoteNever'), d.tailscaleLogin].filter(Boolean).join(' · '))),
            el('button', { class: 'btn btn-small btn-danger', type: 'button', onclick: async () => renderRemote(await window.control.remoteRevoke(d.id)) }, t('remoteRevoke')),
          ),
        )),
  );
}

async function refreshRemote() {
  if (!cfg.remote.enabled) return;
  renderRemote(await window.control.remoteInfo());
}

$('r-enabled').addEventListener('change', async () => {
  const errors = await save({ ...cfg, remote: { ...cfg.remote, enabled: $('r-enabled').checked } }, null);
  if (errors) $('r-enabled').checked = cfg.remote.enabled;
  $('r-body').hidden = !cfg.remote.enabled;
  await refreshRemote();
});

$('r-refresh').addEventListener('click', refreshRemote);

$('r-serve').addEventListener('click', async () => {
  $('r-error').textContent = '';
  const r = await window.control.remoteServe();
  if (!r.ok) $('r-error').textContent = `${t(`err_${r.code}`)}${r.detail ? `: ${r.detail}` : ''}`;
  renderRemote(r.info);
});

$('r-pair').addEventListener('click', async () => {
  $('r-error').textContent = '';
  const r = await window.control.remotePair($('r-name').value);
  if (!r.ok) {
    $('r-error').textContent = t(`err_${r.code}`);
    return;
  }
  $('r-name').value = '';
  $('r-qr').replaceChildren(drawQr(r.qr));
  $('r-url').textContent = r.url;
  $('r-qr-box').hidden = false;
  renderRemote(r.info);
});

$('r-qr-close').addEventListener('click', () => {

  $('r-qr').replaceChildren();
  $('r-url').textContent = '';
  $('r-qr-box').hidden = true;
});

function renderSlots(errors = null) {
  const box = $('slots');
  box.replaceChildren();
  slots.forEach((slot, i) => {
    const days = el('div', { class: 'days' });
    t('days').forEach((name, d) => {
      days.append(
        el(
          'button',
          {
            type: 'button',
            class: slot.days.includes(d) ? 'day day-on' : 'day',
            onclick: () => {
              slot.days = slot.days.includes(d) ? slot.days.filter((x) => x !== d) : [...slot.days, d].sort();
              renderSlots();
            },
          },
          name,
        ),
      );
    });
    const time = (key) => {
      const input = el('input', { type: 'time', value: slot[key] });
      input.addEventListener('change', () => {
        slot[key] = input.value;
      });
      return input;
    };
    const profile = select([['', t('activeProfile')], ...cfg.profiles.map((p) => [p.id, p.name])], slot.profileId ?? '', (v) => (slot.profileId = v || null));
    box.append(
      el(
        'div',
        { class: 'slot' },
        days,
        el('span', { class: 'meta' }, t('from')),
        time('from'),
        el('span', { class: 'meta' }, t('to')),
        time('to'),
        el('div', { class: 'slot-profile' }, profile),
        el('button', { class: 'btn btn-small btn-danger', type: 'button', onclick: () => { slots.splice(i, 1); renderSlots(); } }, t('remove')),
        errText(errors, `slot${i}`) ? el('span', { class: 'error' }, errText(errors, `slot${i}`)) : null,
      ),
    );
  });
}

$('add-slot').addEventListener('click', () => {
  slots.push({ days: [0, 1, 2, 3, 4, 5, 6], from: '00:00', to: '08:00', profileId: null });
  renderSlots();
});

$('save-schedule').addEventListener('click', async () => {
  const errors = await save({ ...cfg, schedule: { enabled: $('s-enabled').checked, slots } }, $('schedule-msg'));
  if (errors) renderSlots(errors);
});

(async () => {
  const init = await window.control.init();
  cfg = init.config;
  coins = init.coins;
  lang = init.lang;
  policy = init.policy;
  defaultCurve = init.defaultCurve;
  viewState = init.view;
  $('subtitle').dataset.base = `by DoriByte · v${init.version}`;
  $('subtitle').textContent = $('subtitle').dataset.base;
  translate();
  remembering();
  $('r-name').placeholder = t('remoteNamePh');
  renderView();
  fillSettings();
  $('s-enabled').checked = cfg.schedule.enabled;
  slots = clone(cfg.schedule.slots);
  renderSlots();
  renderStatus(await window.control.status());
  window.control.onStatus(renderStatus);

  window.control.onConfig((next) => {
    const changed = JSON.stringify(next) !== JSON.stringify(cfg);
    cfg = next;
    if (!changed) return;

    editing = null;
    renderProfiles();
    renderSlots();
  });
})();
