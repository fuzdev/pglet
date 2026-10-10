-- The accept side of `partial_unique_index_member_position`, and the pin that separates
-- "decides membership by the member order" from "indexes everything": `sad` is the
-- **first** member, so `m > 'sad'` excludes both rows and the duplicate `id` is fine.
-- Under a label-byte comparison `sad` is the *largest* label, so the same predicate
-- would index both rows and refuse this insert.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE pt (id INT, m mood NOT NULL);
CREATE UNIQUE INDEX pt_u ON pt (id) WHERE m > 'sad';
INSERT INTO pt VALUES (1, 'sad');
INSERT INTO pt VALUES (1, 'sad');

-- @profile-lock
-- @test
SELECT count(*) AS n FROM pt;
