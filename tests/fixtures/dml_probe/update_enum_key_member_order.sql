-- An index over an enum column is not probed: the enum orders by member position, the key
-- by label bytes. `m > 'sad'` keeps `ok` and `happy` in row order.
-- @profile-lock
-- @setup
CREATE TYPE pmood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE pe (m pmood, v TEXT);
CREATE INDEX pe_m ON pe (m);
INSERT INTO pe VALUES ('happy', 'a'), ('sad', 'b'), ('ok', 'c');
-- @test
UPDATE pe SET v = v || '!' WHERE m > 'sad' RETURNING m::text AS m, v;
