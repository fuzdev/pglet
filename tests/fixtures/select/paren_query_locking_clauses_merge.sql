-- Locking clauses are the one kind the grammar merges however many the query carries.
-- @setup
CREATE TABLE pql_s (x INT);
INSERT INTO pql_s VALUES (1);

-- @test
(SELECT x FROM pql_s FOR UPDATE) FOR UPDATE;
