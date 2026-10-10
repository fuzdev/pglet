-- The same ordering at the statement's own boundary: trailing junk after a locking
-- clause is `42601` on both engines, ahead of pglet's refusal.
-- @expect-error: the trailing-token error outranks the wait-policy refusal
-- @setup
CREATE TABLE lskipj (id INT, code TEXT);
INSERT INTO lskipj VALUES (1, 'a');
-- @test
SELECT id FROM lskipj FOR UPDATE SKIP LOCKED zzz;
