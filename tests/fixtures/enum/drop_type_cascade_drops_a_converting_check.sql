-- `DROP TYPE … CASCADE` drops a `CHECK` converting to the enum — the constraint alone:
-- the table and its rows stay, and a value the enum lacked now writes. It was refused
-- `2BP01`, and with no `DROP CONSTRAINT` the type could not be dropped short of the table.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE mc (id INT, s TEXT CHECK (s::mood IS NOT NULL));
INSERT INTO mc VALUES (1, 'ok');

-- @test
DROP TYPE mood CASCADE;
INSERT INTO mc VALUES (2, 'zzz');
SELECT id, s, (SELECT count(*) FROM pg_constraint WHERE conname = 'mc_s_check') AS checks
FROM mc ORDER BY id;
