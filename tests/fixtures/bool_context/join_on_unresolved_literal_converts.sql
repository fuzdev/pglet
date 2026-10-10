-- `JOIN … ON 't'` is an unconditional join on 18.6; it matched nothing here.
-- @setup
CREATE TABLE bcj (a INT);
INSERT INTO bcj VALUES (1), (2);
-- @test
SELECT count(*) FROM bcj x JOIN bcj y ON 't';
