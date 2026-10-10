-- Several dependents on one table and a second table: `CASCADE` drops both converting
-- `CHECK`s — by position, so the unrelated `CHECK` between them survives — and clears the
-- default; the rows stay.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ms (
  a TEXT CHECK (a::mood IS NOT NULL),
  n INT CHECK (n > 0),
  b TEXT DEFAULT ('ok'::text)::mood CHECK (b::mood <> 'sad')
);
CREATE TABLE ms2 (c TEXT CHECK (c::mood IS NOT NULL));
INSERT INTO ms VALUES ('ok', 1, 'happy');

-- @test
DROP TYPE mood CASCADE;
INSERT INTO ms (a, n) VALUES ('zzz', 2);
INSERT INTO ms2 VALUES ('zzz');
SELECT conname, (SELECT count(*) FROM ms) AS ms_rows,
       (SELECT count(*) FROM ms WHERE b IS NULL) AS no_default
FROM pg_constraint WHERE conname LIKE 'ms%' ORDER BY conname;
