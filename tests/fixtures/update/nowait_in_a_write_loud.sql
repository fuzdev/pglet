-- A `NOWAIT` inside a statement that writes is refused `0A000`: the write takes the
-- writer as every write does, waiting for it, so no host can honour the policy there.
-- 18.7 answers, so any pglet error passes.
-- @expect-error: NOWAIT in a statement that writes not supported
-- @profile-lock
-- @setup
CREATE TABLE lnowait (id INT PRIMARY KEY, code TEXT);
INSERT INTO lnowait VALUES (1, 'a'), (2, 'b');
-- @test
UPDATE lnowait SET code = 'q' WHERE id IN (SELECT id FROM lnowait FOR UPDATE NOWAIT);
