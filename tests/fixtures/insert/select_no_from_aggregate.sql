-- `INSERT … SELECT` of a no-FROM aggregate writes its one row.
-- @setup
CREATE TABLE ins_agg (n bigint);
-- @test
INSERT INTO ins_agg SELECT count(*) RETURNING n;
