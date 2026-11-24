-- `NOWAIT` with no other connection holding the writer: the locking read answers as a
-- plain one does, under each of the four strengths' spellings this one stands for.
-- @profile-lock
-- @setup
CREATE TABLE lnowait (id INT PRIMARY KEY, code TEXT);
INSERT INTO lnowait VALUES (1, 'a'), (2, 'b');
-- @test
SELECT id, code FROM lnowait ORDER BY id FOR SHARE NOWAIT;
