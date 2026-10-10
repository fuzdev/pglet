-- The refusal polarity of `window_derived_table_repeated_name_non_enum_literal.sql`. The
-- derived table's `x.m` is `wtt`'s `text` column, so a `UNION` with an enum column has no
-- common type — 18.6's `42804`. With the window arm's bare-name lookup `x.m` carried
-- `wte`'s enum, the branches unified, and the statement answered both rows.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE wte (m mood NOT NULL);
CREATE TABLE wtt (m TEXT NOT NULL);
INSERT INTO wte VALUES ('happy');
INSERT INTO wtt VALUES ('sad');

-- @test
-- @expect-error: UNION types text and mood cannot be matched
SELECT x.m FROM (SELECT b.m, row_number() OVER () AS rn FROM wte a, wtt b) x
UNION SELECT m FROM wte;
