-- A `SKIP LOCKED` inside a statement that writes is refused `0A000`: the write takes the
-- writer as every write does, waiting for it, so no host can skip what another block
-- holds. 18.7 answers, so any pglet error passes.
-- @expect-error: SKIP LOCKED in a statement that writes not supported
-- @profile-lock
-- @setup
CREATE TABLE lskip (id INT PRIMARY KEY, code TEXT);
INSERT INTO lskip VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @test
UPDATE lskip SET code = 'q' WHERE id IN (SELECT id FROM lskip ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED);
