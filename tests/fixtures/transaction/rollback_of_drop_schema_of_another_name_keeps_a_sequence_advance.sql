-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- `DROP SCHEMA IF EXISTS` of a schema that does not exist drops nothing, so it replaces
-- no cell and the block's later draw stands after the rollback: 18.6 answers `5`.
-- Listing every sequence for any `DROP SCHEMA` answers `4` again.
-- @setup
DROP SEQUENCE IF EXISTS rds_other_s;
CREATE SEQUENCE rds_other_s;
SELECT nextval('rds_other_s');
SELECT nextval('rds_other_s');
SELECT nextval('rds_other_s');
BEGIN;
DROP SCHEMA IF EXISTS rds_nosuch_schema CASCADE;
SELECT nextval('rds_other_s');
ROLLBACK;
-- @test
SELECT nextval('rds_other_s');
