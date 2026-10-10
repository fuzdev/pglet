-- Refusing `json` as a *type* name leaves it an ordinary *column* name. This one is a
-- live differential comparison rather than a documentary fixture: both engines answer,
-- and it reddens on the over-correction that promotes the word to a keyword — which 18.6
-- itself does (`json` is a COL_NAME_KEYWORD there) without losing the identifier.
-- @setup
CREATE TABLE json_names (json INT, jsonb INT);
INSERT INTO json_names VALUES (1, 2);

-- @test
SELECT json, jsonb FROM json_names WHERE json = 1;
