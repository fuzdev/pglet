-- An ordered index read over a `bytea` key holds the empty value and a zero byte, the
-- least keys a value has, in order.
-- @profile-lock
-- @setup
CREATE TABLE lk (c BYTEA NOT NULL, v TEXT);
CREATE INDEX lk_c ON lk (c);
INSERT INTO lk VALUES ('\x01', 'one'), ('\x', 'empty'), ('\x0000', 'two zero bytes'), ('\x00', 'zero byte');
-- @test
SELECT v FROM lk ORDER BY c;
