-- `SQL_ASCII` is Postgres's "no conversion — the bytes pass through", which is exactly
-- what pglet does, so it is the one member of the otherwise-refused encoding set whose
-- refusal would buy nothing. It also has a consumer: `pg_dump` of a `SQL_ASCII` database
-- writes `SET client_encoding = 'SQL_ASCII';` into its preamble, and restoring a
-- plain-text dump is both a gate and a README claim.
--
-- The `SET` is followed by the read the preamble protects, so the fixture asserts that
-- the statement was accepted *and* that the connection still answers — a refusal shows
-- up as the setup's error, and an accept that broke the session shows up here.
-- @setup
CREATE TABLE enc_rows (s TEXT);
INSERT INTO enc_rows VALUES ('café');
-- @test
SET client_encoding = 'SQL_ASCII';
SELECT s FROM enc_rows;
