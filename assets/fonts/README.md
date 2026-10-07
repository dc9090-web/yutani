# Vendored fonts (SIL OFL 1.1 — see the OFL-*.txt files)

- B612 Mono Regular/Bold, Michroma Regular — unmodified, from github.com/google/fonts
  (ofl/), fetched from the `main` branch on 2026-10-07.
- NotoSansJP-Yutani.ttf — the Noto Sans JP variable font (also from
  google/fonts `main`, 2026-10-07), instanced at wght=500 and subset to
  "ユタニ重工" with fonttools:

  ```
  fonttools varLib.instancer NotoSansJP[wght].ttf wght=500 --update-name-table -o NotoSansJP-500.ttf
  pyftsubset NotoSansJP-500.ttf --text="ユタニ重工" --output-file=NotoSansJP-Yutani.ttf
  ```

  `--update-name-table` makes name ID 1 "Noto Sans JP Medium" with typographic
  family (ID 16) "Noto Sans JP". It is a Modified Version under the OFL. The Noto
  OFL reserves the name "Source", which "Noto Sans JP" does not use, so keeping
  the family name is permitted.
