use super::*;

#[test]
fn projektion_ist_output_mal_input_und_transponiert_die_zeitachse() {
    // Nicht symmetrisch: W^T oder zeitweise statt kanalweise Latents liefern andere Werte.
    let w = [1., 2., 3., 4.];
    let x = [5., 7., 11., 13., 17., 19.]; // [1,2,3]
    assert_eq!(project(&w, &x, 2, 3).unwrap(), [31., 67., 41., 89., 49., 109.]);
    assert_ne!(project(&[1., 3., 2., 4.], &x, 2, 3).unwrap(), [31., 67., 41., 89., 49., 109.]);
    assert!(project(&w[..3], &x, 2, 3).is_err());
    assert!(project(&w, &x[..5], 2, 3).is_err());
}

#[test]
#[ignore = "PHONON_GGUF und PHONON_ANTON auf lokale Dateien setzen; keine Fixtures im Repo"]
fn anton_entspricht_dem_iphone() {
    // iPhone-Datenhash: ed2cec926ec76828ca94611f69c7466d64083ee705dbc53a8e7be688d36bc4ee.
    // Unterschiedliche F32-Summierung darf davon abweichen; maßgeblich sind die Werte unten.
    let model = std::env::var_os("PHONON_GGUF").expect("PHONON_GGUF fehlt");
    let voice = std::env::var_os("PHONON_ANTON").expect("PHONON_ANTON fehlt");
    let weights = load_weight(Path::new(&model)).unwrap();
    let bytes = encode(&fs::read(voice).unwrap(), &weights).unwrap();
    let n = u64::from_le_bytes(bytes[..8].try_into().unwrap()) as usize;
    let values = bytes[8 + n..].chunks_exact(4)
        .map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect::<Vec<_>>();
    for (start, expected) in [
        (0, vec![0.10447657853364944, -3.062392234802246, 0.26737043261528015,
            -0.5063714981079102, -0.9594208002090454, -0.0802791491150856,
            0.2047739177942276, 0.3849446177482605]),
        (512, vec![-0.025632472708821297, -1.808097243309021,
            -0.06277405470609665, -0.08551596850156784]),
    ] {
        for (i, expected) in expected.into_iter().enumerate() {
            assert!((values[start + i] as f64 - expected).abs() <= 1e-4, "Abweichung bei {}", start + i);
        }
    }
}

#[test]
fn gguf_alignment_metadaten_und_falsche_form() {
    let original = gguf(0);
    let table_end = 24 + 8 + WEIGHT.len() + 4 + 16 + 4 + 8;
    let original_data = (table_end + 31) / 32 * 32;
    for alignment in [0u32, 3, 64, 8192] {
        let mut b = original[..24].to_vec();
        b[16..24].copy_from_slice(&1u64.to_le_bytes());
        let key = "general.alignment";
        b.extend((key.len() as u64).to_le_bytes()); b.extend(key.as_bytes());
        b.extend(4u32.to_le_bytes()); b.extend(alignment.to_le_bytes());
        b.extend(&original[24..table_end]);
        while b.len() % 64 != 0 { b.push(0); }
        b.extend(&original[original_data..]);
        let result = read_weight(&mut std::io::Cursor::new(&b), b.len() as u64);
        assert_eq!(result.is_ok(), alignment == 64);
    }
    let mut b = original.clone();
    let dimension = 24 + 8 + WEIGHT.len() + 4;
    b[dimension..dimension+8].copy_from_slice(&511u64.to_le_bytes());
    assert!(read_weight(&mut std::io::Cursor::new(&b), b.len() as u64).is_err());
}

fn gguf(typ: u32) -> Vec<u8> {
    let mut b = b"GGUF".to_vec();
    b.extend(3u32.to_le_bytes()); b.extend(1u64.to_le_bytes()); b.extend(0u64.to_le_bytes());
    b.extend((WEIGHT.len() as u64).to_le_bytes()); b.extend(WEIGHT.as_bytes());
    b.extend(2u32.to_le_bytes()); b.extend(512u64.to_le_bytes()); b.extend(512u64.to_le_bytes());
    b.extend(typ.to_le_bytes()); b.extend(0u64.to_le_bytes());
    while b.len() % 32 != 0 { b.push(0); }
    for i in 0..512 * 512 {
        let value = if i == 1 { 2f32 } else if i == 512 { 3f32 } else if i % 513 == 0 { 1f32 } else { 0f32 };
        if typ == 30 { b.extend(((value.to_bits() >> 16) as u16).to_le_bytes()); }
        else { b.extend(value.to_le_bytes()); }
    }
    b
}

#[test]
fn gguf_bf16_f32_und_grenzen_mit_gegenproben() {
    for typ in [0, 30] {
        let bytes = gguf(typ);
        let w = read_weight(&mut std::io::Cursor::new(&bytes), bytes.len() as u64).unwrap();
        assert_eq!(w[0], 1.); assert_eq!(w[1], 2.); assert_eq!(w[512], 3.); assert_eq!(w[513], 1.);
        assert!(read_weight(&mut std::io::Cursor::new(&bytes[..bytes.len()-1]), bytes.len() as u64 - 1).is_err());
        let mut bad = bytes.clone(); bad[4] = 1;
        assert!(read_weight(&mut std::io::Cursor::new(&bad), bad.len() as u64).is_err());
        // Tensor-Offset direkt hinter Typ und Dimensionen; außerhalb bzw. nicht ausgerichtet.
        let offset = 24 + 8 + WEIGHT.len() + 4 + 16 + 4;
        for value in [1u64, u64::MAX - 31, bytes.len() as u64 * 32] {
            let mut bad = bytes.clone(); bad[offset..offset+8].copy_from_slice(&value.to_le_bytes());
            assert!(read_weight(&mut std::io::Cursor::new(&bad), bad.len() as u64).is_err());
        }
        let mut bad = bytes.clone(); bad[24..32].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(read_weight(&mut std::io::Cursor::new(&bad), bad.len() as u64).is_err());
        let mut bad = bytes.clone(); bad[32] = b'X';
        assert!(read_weight(&mut std::io::Cursor::new(&bad), bad.len() as u64).is_err());
    }
    let bytes = gguf(1);
    assert!(read_weight(&mut std::io::Cursor::new(&bytes), bytes.len() as u64).is_err());
}

#[test]
fn safetensors_kopf_und_dynamisches_t() {
    for t in [1, 13, 125, 1024] {
        let h = format!(r#"{{"speaker_wavs":{{"dtype":"F32","shape":[1,512,{t}],"data_offsets":[0,{}]}}}}"#, t*512*4);
        let mut b = (h.len() as u64).to_le_bytes().to_vec();
        b.extend(h.as_bytes()); b.extend(vec![0; t*512*4]);
        let mut w = vec![0.; 512*512];
        for i in 0..512 { w[i*512+i] = 1.; }
        let out = encode(&b, &w).unwrap();
        let n = u64::from_le_bytes(out[..8].try_into().unwrap()) as usize;
        let h: Value = serde_json::from_slice(&out[8..8+n]).unwrap();
        assert_eq!(h["emb"]["shape"], json!([1,t,512]));
        assert_eq!(out.len()-8-n, t*512*4);
        if t == 125 { assert_eq!(n, 72); assert_eq!(out.len(), 256080); }
        assert!(encode(&b[..b.len()-1], &w).is_err());
    }
}
