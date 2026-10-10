-- `IF NOT EXISTS` reads the same namespace: a table over a sequence's name is a
-- skip, not an error. What the test reads is that no table was created — reading
-- the sequence instead would be vacuous, since a skip and a wrongly-created table
-- both leave `nextval` answering 1.
-- @setup
CREATE SEQUENCE ns_ine_seq;
CREATE TABLE IF NOT EXISTS ns_ine_seq (a INT);
-- @test
SELECT count(*) AS tables_created FROM information_schema.tables
 WHERE table_name = 'ns_ine_seq';
