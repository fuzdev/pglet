-- length(bytea) is the BYTE count (Postgres has both length(text) and
-- length(bytea)); length('\xdead'::bytea) = 2.
-- @setup
CREATE TABLE lb (id INT, b BYTEA);
INSERT INTO lb VALUES (1, '\xdead'), (2, '\x00'), (3, NULL);
-- @test
SELECT id, length(b) AS len FROM lb ORDER BY id;
