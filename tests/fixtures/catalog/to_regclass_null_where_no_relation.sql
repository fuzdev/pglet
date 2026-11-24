-- Where no relation answers, `to_regclass` is NULL rather than `::regclass`'s `42P01`: a
-- name nothing carries (an unquoted `TrcOdd` folds to `trcodd`), a schema that does not
-- exist, `public.pg_class` (the catalog relation is not in `public`), a malformed name
-- (empty, an unterminated quote, two words), a temporary schema, and NULL.
-- @setup
CREATE TABLE "TrcOdd" (id INT);
-- @test
SELECT to_regclass('trc_nope') IS NULL AS missing,
       to_regclass('TrcOdd') IS NULL AS folded_away,
       to_regclass('nosuch.trc') IS NULL AS no_schema,
       to_regclass('public.pg_class') IS NULL AS not_in_public,
       to_regclass('') IS NULL AS empty,
       to_regclass('"open') IS NULL AS unterminated,
       to_regclass('trc a') IS NULL AS two_words,
       to_regclass('pg_temp.trc') IS NULL AS temp_schema,
       to_regclass(NULL) IS NULL AS null_arg;
