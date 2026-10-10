-- The code that separates this pair from the `text` one, and the reason the rulebook
-- models a **category** rather than a type identity: two enums share category `E`, so
-- `select_common_type` gets past the category check and the *conversion* is what fails —
-- `42846`, where the enum/`text` pair is `42804`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE v (id INT, m mood NOT NULL, y m3 NOT NULL);
INSERT INTO v VALUES (1, 'ok', 'sad'), (2, 'ok', 'happy'), (3, 'ok', 'ok');

-- @test
-- @expect-error: cannot convert m3 to mood
-- @expect-message
SELECT greatest(m, y)::text FROM v ORDER BY id;
