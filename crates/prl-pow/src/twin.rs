//! Gémeo exato, em CPU, de uma tentativa de mineração de Pearl (conta de inteiros, certificados V2 e V3).
//!
//! Escrito de raiz. Conferido contra o verificador oficial (`pearl-research-labs/pearl`, crate `zk-pow`)
//! com vetores gerados a chamar as funções oficiais.
//!
//! Etapas, pela ordem em que acontecem:
//!   1. `chave_do_trabalho`  = BLAKE3(cabeçalho de 76 bytes || configuração de 52 bytes)
//!   2. `raiz`               = BLAKE3 com chave (a do trabalho) sobre a matriz, cheia de zeros até um múltiplo de 1024
//!   3. V3: cada raiz é salgada com a dimensão (m para A, n para B)
//!   4. sementes: B = BLAKE3(chave || raiz_B); A = BLAKE3(semente_B || raiz_A)
//!   5. ruído de característica r: (matriz uniforme m×r) × (matriz k×r com um +1 e um −1 por linha)
//!   6. por tile: soma de produtos em troços de r; no fim de cada troço, XOR de todos os acumuladores
//!      misturado num transcrito de 16 palavras (rotação de 13)
//!   7. BLAKE3 com chave (a semente de A) sobre as 16 palavras; inteiro de 256 bits, little-endian, <= alvo × fator

pub const PALAVRAS_TRANSCRITO: usize = 16;
pub const ROTACAO: u32 = 13;
pub const RANK_BASE_PENALIZACAO: usize = 128;

// ---------------------------------------------------------------------------------------------
// Inteiro de 256 bits sem sinal, o mínimo necessário (quatro palavras de 64 bits, a baixa primeiro).
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct U256(pub [u64; 4]);

impl U256 {
    pub const ZERO: U256 = U256([0; 4]);
    pub const MAX: U256 = U256([u64::MAX; 4]);

    pub fn de_bytes_le(b: &[u8; 32]) -> U256 {
        let mut w = [0u64; 4];
        for (i, p) in w.iter_mut().enumerate() {
            *p = u64::from_le_bytes(b[i * 8..i * 8 + 8].try_into().unwrap());
        }
        U256(w)
    }

    pub fn para_bytes_le(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        for i in 0..4 {
            b[i * 8..i * 8 + 8].copy_from_slice(&self.0[i].to_le_bytes());
        }
        b
    }

    /// Desloca à esquerda; os bits que saem por cima perdem-se.
    pub fn desloca_esq(&self, bits: u32) -> U256 {
        if bits >= 256 {
            return U256::ZERO;
        }
        let (palavras, resto) = ((bits / 64) as usize, bits % 64);
        let mut w = [0u64; 4];
        for i in (palavras..4).rev() {
            let origem = i - palavras;
            let mut v = self.0[origem] << resto;
            if resto != 0 && origem > 0 {
                v |= self.0[origem - 1] >> (64 - resto);
            }
            w[i] = v;
        }
        U256(w)
    }

    /// Multiplica por um fator de 64 bits; `None` se o resultado não couber em 256 bits.
    pub fn vezes(&self, fator: u64) -> Option<U256> {
        let mut w = [0u64; 4];
        let mut transporte: u128 = 0;
        for i in 0..4 {
            let p = self.0[i] as u128 * fator as u128 + transporte;
            w[i] = p as u64;
            transporte = p >> 64;
        }
        if transporte != 0 {
            None
        } else {
            Some(U256(w))
        }
    }

    pub fn menor_ou_igual(&self, outro: &U256) -> bool {
        for i in (0..4).rev() {
            if self.0[i] != outro.0[i] {
                return self.0[i] < outro.0[i];
            }
        }
        true
    }
}

/// Alvo compacto ao estilo Bitcoin: mantissa de 3 bytes vezes 256^(expoente − 3).
/// Mantissa nula, expoente nulo ou bit de sinal (0x00800000) dão zero.
pub fn alvo_de_nbits(nbits: u32) -> U256 {
    let expoente = nbits >> 24;
    let mantissa = nbits & 0x00ff_ffff;
    if mantissa == 0 || expoente == 0 || mantissa & 0x0080_0000 != 0 {
        return U256::ZERO;
    }
    if expoente <= 3 {
        U256([(mantissa >> (8 * (3 - expoente))) as u64, 0, 0, 0])
    } else {
        U256([mantissa as u64, 0, 0, 0]).desloca_esq(8 * (expoente - 3))
    }
}

// ---------------------------------------------------------------------------------------------
// Cabeçalho, padrões e configuração
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Cabecalho {
    pub versao: u32,
    pub bloco_anterior: [u8; 32],
    pub raiz_transacoes: [u8; 32],
    pub tempo: u32,
    pub nbits: u32,
}

impl Cabecalho {
    /// 76 bytes: campos em little-endian e os dois hashes com os bytes invertidos.
    pub fn bytes(&self) -> [u8; 76] {
        let mut b = [0u8; 76];
        b[0..4].copy_from_slice(&self.versao.to_le_bytes());
        for i in 0..32 {
            b[4 + i] = self.bloco_anterior[31 - i];
            b[36 + i] = self.raiz_transacoes[31 - i];
        }
        b[68..72].copy_from_slice(&self.tempo.to_le_bytes());
        b[72..76].copy_from_slice(&self.nbits.to_le_bytes());
        b
    }
}

/// Padrão periódico de índices: soma direta de até três progressões (passo, comprimento),
/// da mais fina para a mais larga. Ex.: {0, 8, 64, 72} = (8, 2) + (64, 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Padrao {
    pub dims: Vec<(u32, u32)>,
}

impl Padrao {
    /// Decompõe uma lista crescente que começa em 0. Em cada nível toma a corrida mais longa
    /// de múltiplos do primeiro passo (a forma canónica: um passo não se parte em dois níveis).
    pub fn de_lista(lista: &[u32]) -> Result<Padrao, String> {
        if lista.is_empty() || lista[0] != 0 {
            return Err("o padrão tem de começar em 0".into());
        }
        if !lista.windows(2).all(|p| p[0] < p[1]) {
            return Err("o padrão tem de ser estritamente crescente".into());
        }
        let mut dims = Vec::new();
        let mut resto: Vec<u32> = lista.to_vec();
        while resto.len() > 1 {
            let passo = resto[1];
            let mut comprimento = 1usize;
            while comprimento < resto.len() && resto[comprimento] == passo * comprimento as u32 {
                comprimento += 1;
            }
            if resto.len() % comprimento != 0 {
                return Err("o padrão não é uma soma direta de progressões".into());
            }
            let anterior = dims.last().map_or(1, |d: &(u32, u32)| d.0 * d.1);
            if passo % anterior != 0 {
                return Err("o passo de um nível tem de ser múltiplo do período do nível anterior".into());
            }
            dims.push((passo, comprimento as u32));
            resto = resto.iter().step_by(comprimento).copied().collect();
        }
        if dims.len() > 3 {
            return Err("o padrão tem mais de três níveis".into());
        }
        let p = Padrao { dims };
        if p.indices() != lista {
            return Err("o padrão não é uma soma direta de progressões".into());
        }
        Ok(p)
    }

    /// Os índices do padrão, por ordem crescente.
    pub fn indices(&self) -> Vec<u32> {
        let mut v = vec![0u32];
        for &(passo, comprimento) in &self.dims {
            let mut novo = Vec::with_capacity(v.len() * comprimento as usize);
            for i in 0..comprimento {
                novo.extend(v.iter().map(|x| x + i * passo));
            }
            v = novo;
        }
        v
    }

    pub fn tamanho(&self) -> usize {
        self.dims.iter().map(|d| d.1 as usize).product()
    }

    pub fn periodo(&self) -> u32 {
        self.dims.last().map_or(1, |d| d.0 * d.1)
    }

    /// Uma base é válida se, em cada nível, cair na primeira fatia do período desse nível.
    /// As bases válidas de um período, com o padrão somado, cobrem o período sem sobreposição.
    pub fn base_valida(&self, base: u32) -> bool {
        self.dims.iter().all(|&(passo, comprimento)| base % (passo * comprimento) < passo)
    }

    /// 6 bytes: por nível, (passo / período do nível anterior − 1, comprimento − 1); níveis em falta a zero.
    pub fn bytes(&self) -> [u8; 6] {
        let mut b = [0u8; 6];
        let mut anterior = 1u32;
        for (i, &(passo, comprimento)) in self.dims.iter().enumerate() {
            b[2 * i] = (passo / anterior - 1) as u8;
            b[2 * i + 1] = (comprimento - 1) as u8;
            anterior = passo * comprimento;
        }
        b
    }
}

#[derive(Clone, Debug)]
pub struct Configuracao {
    pub k: u32,
    pub rank: u16,
    pub linhas: Padrao,
    pub colunas: Padrao,
}

impl Configuracao {
    /// 52 bytes: k (4), rank (2), tipo de conta (2, sempre 0), os dois padrões (6 + 6), 32 a zero (sem MoE).
    pub fn bytes(&self) -> [u8; 52] {
        let mut b = [0u8; 52];
        b[0..4].copy_from_slice(&self.k.to_le_bytes());
        b[4..6].copy_from_slice(&self.rank.to_le_bytes());
        b[8..14].copy_from_slice(&self.linhas.bytes());
        b[14..20].copy_from_slice(&self.colunas.bytes());
        b
    }

    /// Comprimento do produto interno que conta: k arredondado para baixo a um múltiplo de r.
    pub fn comprimento_util(&self) -> usize {
        let (k, r) = (self.k as usize, self.rank as usize);
        k - k % r
    }

    pub fn h(&self) -> usize {
        self.linhas.tamanho()
    }
    pub fn w(&self) -> usize {
        self.colunas.tamanho()
    }

    /// Fator pelo qual o alvo é multiplicado: acumuladores do tile × comprimento do produto.
    pub fn fator(&self) -> u64 {
        (self.h() * self.w() * self.comprimento_util()) as u64
    }

    /// Com a regra de penalização: acumuladores × número de troços × 128.
    pub fn fator_penalizado(&self) -> u64 {
        (self.h() * self.w() * (self.comprimento_util() / self.rank as usize) * RANK_BASE_PENALIZACAO) as u64
    }
}

/// Limite do hash sem penalização; satura no máximo quando não cabe.
pub fn limite(nbits: u32, c: &Configuracao) -> U256 {
    alvo_de_nbits(nbits).vezes(c.fator()).unwrap_or(U256::MAX)
}

/// Limite com penalização de rank; `None` quando não cabe em 256 bits ou o fator é nulo.
pub fn limite_penalizado(nbits: u32, c: &Configuracao) -> Option<U256> {
    let f = c.fator_penalizado();
    if f == 0 {
        return None;
    }
    alvo_de_nbits(nbits).vezes(f)
}

// ---------------------------------------------------------------------------------------------
// Compromissos e sementes
// ---------------------------------------------------------------------------------------------

pub fn chave_do_trabalho(cab: &Cabecalho, cfg: &Configuracao) -> [u8; 32] {
    let mut d = Vec::with_capacity(128);
    d.extend_from_slice(&cab.bytes());
    d.extend_from_slice(&cfg.bytes());
    *blake3::hash(&d).as_bytes()
}

/// Raiz do compromisso de uma matriz dada por linhas (para B: as colunas de B como linhas).
/// A árvore de Merkle é a própria árvore do BLAKE3 com chave, sobre os bytes cheios de zeros até 1024.
pub fn raiz(matriz: &[i8], chave: &[u8; 32]) -> [u8; 32] {
    let cheio = (matriz.len() + 1023) / 1024 * 1024;
    let mut h = blake3::Hasher::new_keyed(chave);
    // i8 e u8 têm a mesma representação.
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(matriz.as_ptr() as *const u8, matriz.len()) };
    h.update(bytes);
    if cheio > matriz.len() {
        h.update(&vec![0u8; cheio - matriz.len()]);
    }
    *h.finalize().as_bytes()
}

const CONTEXTO_SAL_A: &[u8] = b"pearl/cert-v3/noise-seed/A";
const CONTEXTO_SAL_B: &[u8] = b"pearl/cert-v3/noise-seed/B";

/// V3: BLAKE3 com chave (o sal) sobre um bloco de 64 bytes: raiz, dimensão em 4 bytes, zeros.
pub fn raiz_salgada(raiz: &[u8; 32], dimensao: u32, lado_a: bool) -> [u8; 32] {
    let sal = *blake3::hash(if lado_a { CONTEXTO_SAL_A } else { CONTEXTO_SAL_B }).as_bytes();
    let mut msg = [0u8; 64];
    msg[..32].copy_from_slice(raiz);
    msg[32..36].copy_from_slice(&dimensao.to_le_bytes());
    *blake3::keyed_hash(&sal, &msg).as_bytes()
}

/// (semente do ruído de B, semente do ruído de A). `v3` escolhe as raízes salgadas.
pub fn sementes(chave: &[u8; 32], raiz_a: &[u8; 32], raiz_b: &[u8; 32], m: u32, n: u32, v3: bool) -> ([u8; 32], [u8; 32]) {
    let (ra, rb) = if v3 {
        (raiz_salgada(raiz_a, m, true), raiz_salgada(raiz_b, n, false))
    } else {
        (*raiz_a, *raiz_b)
    };
    let mut d = [0u8; 64];
    d[..32].copy_from_slice(chave);
    d[32..].copy_from_slice(&rb);
    let semente_b = *blake3::hash(&d).as_bytes();
    d[..32].copy_from_slice(&semente_b);
    d[32..].copy_from_slice(&ra);
    let semente_a = *blake3::hash(&d).as_bytes();
    (semente_b, semente_a)
}

// ---------------------------------------------------------------------------------------------
// Ruído
// ---------------------------------------------------------------------------------------------

/// Um bloco de 32 bytes pseudoaleatórios: BLAKE3 com chave (a semente) sobre 64 bytes em que o
/// contador (índice + 1) ocupa a palavra `posicao` e a etiqueta do lado ocupa os bytes 32 a 39.
fn bloco_aleatorio(indice: usize, etiqueta: &[u8; 8], semente: &[u8; 32], posicao: usize) -> [u8; 32] {
    let mut msg = [0u8; 64];
    msg[posicao * 4..posicao * 4 + 4].copy_from_slice(&((indice + 1) as i32).to_le_bytes());
    msg[32..40].copy_from_slice(etiqueta);
    *blake3::keyed_hash(semente, &msg).as_bytes()
}

/// Uma linha da matriz uniforme (r valores entre −32 e 31): 6 bits de cada byte, menos 32.
pub fn linha_uniforme(linha: usize, r: usize, etiqueta: &[u8; 8], semente: &[u8; 32], blocos: &mut u64) -> Vec<i8> {
    let inicio = linha * r;
    let mut v = Vec::with_capacity(r);
    let mut bloco = inicio / 32;
    while v.len() < r {
        let bytes = bloco_aleatorio(bloco, etiqueta, semente, 0);
        *blocos += 1;
        for (j, &byte) in bytes.iter().enumerate() {
            let i = bloco * 32 + j;
            if i >= inicio && i < inicio + r {
                v.push((byte & 63) as i8 - 32);
            }
        }
        bloco += 1;
    }
    v
}

/// Para cada uma das k posições, dois índices distintos em [0, r): o do +1 e o do −1.
pub fn pares(k: usize, r: usize, etiqueta: &[u8; 8], semente: &[u8; 32], blocos: &mut u64) -> Vec<(u32, u32)> {
    let mascara = (r - 1) as u32;
    let mut v = Vec::with_capacity(k);
    let mut bloco = 0usize;
    while v.len() < k {
        let bytes = bloco_aleatorio(bloco, etiqueta, semente, 1);
        *blocos += 1;
        for j in 0..8 {
            if v.len() == k {
                break;
            }
            let u = u32::from_le_bytes(bytes[j * 4..j * 4 + 4].try_into().unwrap());
            let primeiro = u & mascara;
            // parte alta de (r − 1) × u: um número em [0, r − 2]; mais um, nunca é zero
            let salto = 1 + ((mascara as u64 * u as u64) >> 32) as u32;
            v.push((primeiro, primeiro ^ salto));
        }
        bloco += 1;
    }
    v
}

pub struct Ruido {
    /// `linhas` × k, por linhas.
    pub valores: Vec<i8>,
    pub blocos_blake3: u64,
}

/// Ruído de um lado para as linhas pedidas (para B: as colunas de B). Lado A usa a semente de A.
pub fn ruido(lado_a: bool, semente: &[u8; 32], linhas: &[usize], k: usize, r: usize) -> Ruido {
    let etiqueta: &[u8; 8] = if lado_a { b"A_tensor" } else { b"B_tensor" };
    let mut blocos = 0u64;
    let p = pares(k, r, etiqueta, semente, &mut blocos);
    let mut valores = Vec::with_capacity(linhas.len() * k);
    for &linha in linhas {
        let u = linha_uniforme(linha, r, etiqueta, semente, &mut blocos);
        valores.extend(p.iter().map(|&(mais, menos)| u[mais as usize] - u[menos as usize]));
    }
    Ruido { valores, blocos_blake3: blocos }
}

// ---------------------------------------------------------------------------------------------
// A tentativa
// ---------------------------------------------------------------------------------------------

#[derive(Default, Clone, Copy, Debug)]
pub struct Contagem {
    pub produtos: u64,
    pub palavras_xor: u64,
    pub reducoes: u64,
    pub compressoes_finais: u64,
}

/// Transcrito de um tile. `a` tem h linhas e `b` tem w linhas (colunas de B), todas de comprimento k,
/// já com o ruído somado.
pub fn transcrito(a: &[&[i8]], b: &[&[i8]], k: usize, r: usize, c: &mut Contagem) -> [u32; PALAVRAS_TRANSCRITO] {
    let (h, w) = (a.len(), b.len());
    let mut acumuladores = vec![0i32; h * w];
    let mut t = [0u32; PALAVRAS_TRANSCRITO];
    let trocos = k / r;
    for troco in 0..trocos {
        let (de, ate) = (troco * r, troco * r + r);
        let mut x = 0u32;
        for u in 0..h {
            for v in 0..w {
                let mut s = acumuladores[u * w + v];
                for l in de..ate {
                    s = s.wrapping_add(a[u][l] as i32 * b[v][l] as i32);
                }
                acumuladores[u * w + v] = s;
                x ^= s as u32;
            }
        }
        c.produtos += (h * w * r) as u64;
        c.palavras_xor += (h * w) as u64;
        c.reducoes += 1;
        let i = troco % PALAVRAS_TRANSCRITO;
        t[i] = t[i].rotate_left(ROTACAO) ^ x;
    }
    t
}

/// A única compressão BLAKE3 por tentativa: 64 bytes, chave = semente do ruído de A.
pub fn hash_do_transcrito(t: &[u32; PALAVRAS_TRANSCRITO], semente_a: &[u8; 32], c: &mut Contagem) -> [u8; 32] {
    let mut msg = [0u8; 64];
    for (i, p) in t.iter().enumerate() {
        msg[i * 4..i * 4 + 4].copy_from_slice(&p.to_le_bytes());
    }
    c.compressoes_finais += 1;
    *blake3::keyed_hash(semente_a, &msg).as_bytes()
}

pub fn ganha(hash: &[u8; 32], limite: &U256) -> bool {
    U256::de_bytes_le(hash).menor_ou_igual(limite)
}

/// Soma o ruído à matriz (os valores cabem sempre em int8: |64| + |63| <= 127).
pub fn soma_ruido(matriz: &[i8], ruido: &[i8]) -> Vec<i8> {
    matriz.iter().zip(ruido).map(|(&x, &e)| (x as i32 + e as i32) as i8).collect()
}

/// Compressões de BLAKE3 para resumir `bytes` bytes de uma vez (árvore incluída).
pub fn compressoes_blake3(bytes: u64) -> u64 {
    if bytes == 0 {
        return 1;
    }
    let pedacos = (bytes + 1023) / 1024;
    let blocos = (bytes + 63) / 64;
    blocos + (pedacos - 1)
}

/// O que o verificador aceita, pela ordem em que o verifica. Devolve o motivo da recusa.
/// `consenso` junta a regra da rede principal (rank >= 128), que é do nó e não do verificador puro.
pub fn forma_legal(cfg: &Configuracao, m: u32, n: u32, consenso: bool) -> Result<(), String> {
    let (k, r, h, w) = (cfg.k as usize, cfg.rank as usize, cfg.h(), cfg.w());
    if !(r.is_power_of_two() && (32..=1024).contains(&r)) {
        return Err(format!("rank tem de ser potência de 2 entre 32 e 1024 (r={r})"));
    }
    if k > 1 << 16 {
        return Err(format!("k <= 65536 (k={k})"));
    }
    if k % 64 != 0 {
        return Err(format!("k múltiplo de 64 (k={k})"));
    }
    if k > 4 * r * r {
        return Err(format!("k <= 4r² (k={k}, r={r})"));
    }
    if k < 16 * r {
        return Err(format!("k >= 16r (k={k}, r={r})"));
    }
    if k < 1024 {
        return Err(format!("k >= 1024 (k={k})"));
    }
    if h % 2 != 0 || w % 2 != 0 {
        return Err(format!("h e w pares (h={h}, w={w})"));
    }
    if h * w < 32 || h * w > 256 {
        return Err(format!("32 <= h·w <= 256 (h={h}, w={w})"));
    }
    if m > 1 << 24 || n > 1 << 24 {
        return Err("m, n <= 2^24".into());
    }
    if (h + w) * cfg.comprimento_util() > 1 << 22 {
        return Err("(h + w) × comprimento útil <= 4 MiB".into());
    }
    if consenso && r < RANK_BASE_PENALIZACAO {
        return Err(format!("regra de penalização: rank >= 128 (r={r})"));
    }
    Ok(())
}
