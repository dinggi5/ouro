// ouro-parse 의 C 문(`ouro-parse/src/ffi.rs`) — 스위프트 래퍼 `OuroParse` 만 부른다.
#ifndef OURO_PARSE_H
#define OURO_PARSE_H

/// 「내일 3시 치과」 → 초안 JSON. now = 로컬 `YYYY-MM-DDTHH:MM:SS`, base = `YYYY-MM-DD` 또는 NULL.
/// 입력이 틀렸으면 NULL. 돌려받은 글은 `ouro_free` 로 푼다.
char *ouro_parse(const char *text, const char *now, const char *base);
void ouro_free(char *p);

#endif
