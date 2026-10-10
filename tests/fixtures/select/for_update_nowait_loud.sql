-- `NOWAIT`'s half of the same refusal: it promises an immediate `55P03` where a row is
-- already locked, which an engine that takes no row locks can never raise. 18.6 accepts,
-- so any pglet error passes.
-- @expect-error: NOWAIT names a behaviour pglet cannot produce
-- @setup
CREATE TABLE lnowait (id INT, code TEXT);
INSERT INTO lnowait VALUES (1, 'a'), (2, 'b');
-- @test
SELECT id, code FROM lnowait ORDER BY id FOR SHARE NOWAIT;
