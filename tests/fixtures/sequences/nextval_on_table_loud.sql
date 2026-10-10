-- `nextval` opens its relation through `init_sequence`, which validates the kind: a
-- table name is 42809, not the 42P01 a sequences-map-only lookup reports.
-- @setup
CREATE TABLE nvt_host (a INT);
-- @test
SELECT nextval('nvt_host');
-- @expect-error: cannot open relation, not a sequence (42809)
