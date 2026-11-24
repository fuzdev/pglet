-- Locking items with different wait policies in one statement are refused `0A000`: the
-- writer standing in for every row lock is one lock, which a statement either waits for
-- or does not. 18.7 waits for one item's rows and not the other's, so any pglet error
-- passes.
-- @expect-error: a locking clause mixing wait policies not supported
-- @profile-lock
-- @setup
CREATE TABLE lnowait (id INT PRIMARY KEY, code TEXT);
INSERT INTO lnowait VALUES (1, 'a'), (2, 'b');
-- @test
SELECT id FROM lnowait ORDER BY id FOR UPDATE NOWAIT FOR SHARE;
