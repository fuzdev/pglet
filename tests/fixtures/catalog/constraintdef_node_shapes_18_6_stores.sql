-- Four parse-analysis shapes `pg_get_constraintdef` reconstructs, each measured on 18.6
-- and each a *different* node from the one pglet's parser built.
--
-- `q5`–`q7`: 18.6 has **no** `IS NOT DISTINCT FROM` node. `transformAExprDistinct`
-- builds the `DistinctExpr` and wraps the negated spelling in a `NOT`, so the constraint
-- reads back `(NOT (a IS DISTINCT FROM 1))`. `q8` is the agreeing polarity.
--
-- `q9`: `generate_function_name` runs the name through `quote_identifier`, so a function
-- whose name is a keyword comes back quoted — `substring` is a `col_name_keyword`. The
-- `SUBSTRING(s FROM 1 FOR 2)` spelling of the same call is 18.6's `COERCE_SQL_SYNTAX`
-- memory of how it was *written*, which pglet does not store and does not reproduce.
--
-- `q10`: `concat_ws(text, VARIADIC "any")` declares `text` for its **first** argument
-- only, so that one unknown resolves and the variadic ones do not. `q11`/`q12` are the
-- neighbours that resolve nothing (`concat` is `VARIADIC "any"` throughout) and
-- everything (`upper(text)`).
--
-- `q1`–`q4`: the `EXTRACT` / `date_part` field is a `text` `Const` on 18.6, printed
-- through `quote_identifier` by the `F_EXTRACT_TEXT_*` arm — so `'MONTH'` comes back
-- `"MONTH"`, its case kept inside the quotes, and a lowercase field bare — and as an
-- ordinary labelled argument by `date_part`'s. `extract_field_quoting_*.sql` holds the
-- quoting rule's members.
-- @setup
CREATE TABLE ff (a INT, s TEXT, t TIMESTAMPTZ, n NUMERIC);
ALTER TABLE ff ADD CONSTRAINT q1 CHECK (EXTRACT('year' FROM t) > 2000.0);
ALTER TABLE ff ADD CONSTRAINT q2 CHECK (EXTRACT('year' FROM t) > n);
ALTER TABLE ff ADD CONSTRAINT q3 CHECK (EXTRACT('MONTH' FROM t) = EXTRACT('day' FROM t));
ALTER TABLE ff ADD CONSTRAINT q4 CHECK (date_part('year', t) = date_part('day', t));
ALTER TABLE ff ADD CONSTRAINT q5 CHECK (a IS NOT DISTINCT FROM 1);
ALTER TABLE ff ADD CONSTRAINT q6 CHECK (s IS NOT DISTINCT FROM 'x');
ALTER TABLE ff ADD CONSTRAINT q7 CHECK (NOT (a IS NOT DISTINCT FROM 1));
ALTER TABLE ff ADD CONSTRAINT q8 CHECK (a IS DISTINCT FROM 1);
ALTER TABLE ff ADD CONSTRAINT q9 CHECK (substring(s, 1, 2) = 'ab');
ALTER TABLE ff ADD CONSTRAINT q10 CHECK (concat_ws('-', s, 'x') = 'ab');
ALTER TABLE ff ADD CONSTRAINT q11 CHECK (concat(s, 'x') = 'ab');
ALTER TABLE ff ADD CONSTRAINT q12 CHECK (upper(s) = 'AB');
-- @test
SELECT c.conname::text AS conname, pg_get_constraintdef(c.oid) AS definition
FROM pg_constraint c JOIN pg_class cls ON cls.oid = c.conrelid
WHERE cls.relname::text = 'ff' AND c.contype::text = 'c'
ORDER BY c.conname::text ASC
