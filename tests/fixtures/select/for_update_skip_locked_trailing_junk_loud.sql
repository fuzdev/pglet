-- Trailing junk after a `SKIP LOCKED` locking clause is the statement's own `42601` on
-- both engines.
-- @expect-error: the trailing-token error
-- @setup
CREATE TABLE lskipj (id INT, code TEXT);
INSERT INTO lskipj VALUES (1, 'a');
-- @test
SELECT id FROM lskipj FOR UPDATE SKIP LOCKED zzz;
