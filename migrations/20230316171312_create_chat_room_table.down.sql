-- Add down migration script here
ALTER TABLE messages DROP CONSTRAINT fk_chat_room;
ALTER TABLE messages DROP COLUMN chat_room_id;
ALTER TABLE clans DROP CONSTRAINT fk_chat_room;
ALTER TABLE clans DROP COLUMN chat_room_id;
DROP TABLE chat_rooms;