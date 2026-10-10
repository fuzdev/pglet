-- A partial index's `WHERE` is a **persisted** predicate, so a parenless `char`
-- constant inside it decided which rows the index held: under the truncation the
-- predicate read `c = 'a'`, so the index carried the wrong row and a query matching
-- the predicate read through it. Both rows are present, so a lost rule answers `a`
-- rather than erroring.
-- @profile-lock
-- @setup
CREATE TABLE pic (c TEXT);
INSERT INTO pic VALUES ('ab'), ('a');
CREATE INDEX pic_i ON pic (c) WHERE c = char 'ab';
-- @test
SELECT c FROM pic WHERE c = char 'ab';
