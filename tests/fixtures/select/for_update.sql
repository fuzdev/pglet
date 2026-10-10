-- @setup
CREATE TABLE invite (id INT, code TEXT, claimed BOOLEAN);
INSERT INTO invite VALUES (1, 'a', false), (2, 'b', false), (3, 'c', true);

-- @test
-- Inside a transaction block — the harness runs this fixture in one — `FOR UPDATE`
-- takes pglet's writer (one block at a time stages writes), which only a second
-- connection can see; to this one the result matches Postgres'.
-- This is the spine's find-and-claim shape (`SELECT … WHERE … FOR UPDATE`).
SELECT id, code FROM invite WHERE NOT claimed ORDER BY id ASC FOR UPDATE;
