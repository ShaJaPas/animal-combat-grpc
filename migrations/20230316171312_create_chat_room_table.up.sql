-- Add up migration script here
CREATE TABLE chat_rooms
(
    id SERIAL PRIMARY KEY
);

ALTER TABLE messages ADD chat_room_id INTEGER NOT NULL;
ALTER TABLE messages ADD CONSTRAINT fk_chat_room FOREIGN KEY(chat_room_id) REFERENCES chat_rooms(id);
ALTER TABLE clans ADD chat_room_id INTEGER NOT NULL;
ALTER TABLE clans ADD CONSTRAINT fk_chat_room FOREIGN KEY(chat_room_id) REFERENCES chat_rooms(id);