-- A `SET` value written as an integer literal is the integer, as 18.6's grammar builds it
-- (`NumericOnly`'s `ICONST`): `007` is `7`, and `0_1` is `1`.
-- @setup
SELECT 1;
SET application_name = 007;
DROP TABLE IF EXISTS guc_int_seen;
CREATE TABLE guc_int_seen (v TEXT);
INSERT INTO guc_int_seen SELECT current_setting('application_name');
SET application_name = 0_1;
INSERT INTO guc_int_seen SELECT current_setting('application_name');
-- @test
SELECT v FROM guc_int_seen ORDER BY v;
