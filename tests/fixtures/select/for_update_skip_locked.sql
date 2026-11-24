-- `SKIP LOCKED` with no other connection holding the writer: nothing is locked, so
-- nothing is skipped — the worker-queue claim answers the first row.
-- @profile-lock
-- @setup
CREATE TABLE lskip (id INT PRIMARY KEY, code TEXT);
INSERT INTO lskip VALUES (1, 'a'), (2, 'b'), (3, 'c');
-- @test
SELECT id, code FROM lskip ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED;
