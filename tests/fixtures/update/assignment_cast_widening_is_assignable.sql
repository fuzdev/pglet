-- **A false-rejection control — do not trim it.** The pairs `pg_cast` registers as
-- implicit or assignment all still assign, and they are most of what ordinary SQL does:
-- the numeric family in every pairing, the date/timestamp family in every pairing, and
-- every type at all into a string-category column. A gate that refused any of these
-- would break far more than it fixes.
-- @setup
CREATE TABLE acw (id INT, i INT, n NUMERIC, ts TIMESTAMPTZ, d DATE, t TEXT);
INSERT INTO acw (id) VALUES (1);
-- @test
UPDATE acw
SET i = 5::BIGINT,
    n = 5::INT,
    ts = '2024-03-01'::DATE,
    d = '2024-03-01 10:00:00+00'::TIMESTAMPTZ,
    t = 42
WHERE id = 1
RETURNING i, n, ts, d, t;
