-- `SKIP LOCKED` is refused (`0A000`) rather than honored as a no-op. 18.6 **accepts**
-- it, so this is a deliberately pglet-stricter fixture: any pglet error passes. It
-- cannot be a no-op: a locking read inside a block takes the writer, so a second
-- connection's `SKIP LOCKED` read in its own block would wait for the first block, or
-- answer `55P03` at the bound, where 18.6 hands it the next unlocked row at once.
-- @expect-error: SKIP LOCKED names a behaviour pglet cannot produce
-- @setup
CREATE TABLE lskip (id INT, code TEXT);
INSERT INTO lskip VALUES (1, 'a'), (2, 'b');
-- @test
SELECT id, code FROM lskip ORDER BY id LIMIT 1 FOR UPDATE SKIP LOCKED;
