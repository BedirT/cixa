import json
import os
import socket
import tempfile
import threading
import unittest
from pathlib import Path

from cixa import CixaClient


class ClientTests(unittest.TestCase):
    def test_v1_line_protocol_and_file_token(self):
        with tempfile.TemporaryDirectory() as directory:
            socket_path = os.path.join(directory, "broker.sock")
            token_path = os.path.join(directory, "agent.token")
            Path(token_path).write_text("synthetic-token\n", encoding="utf-8")
            listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            listener.bind(socket_path)
            listener.listen(2)
            operations = []

            def serve():
                for _ in range(2):
                    channel, _ = listener.accept()
                    with channel:
                        request = json.loads(channel.recv(65536).decode("utf-8"))
                        self.assertEqual(request["api_version"], "v1")
                        self.assertEqual(request["token"], "synthetic-token")
                        operations.append(request["operation"])
                        data = ({"transactions": [], "transactions_total": 0, "next_cursor": None, "has_more": False}
                                if request["operation"]["type"] == "list_transactions_page"
                                else {"principal": "agent"})
                        channel.sendall((json.dumps({"api_version": "v1", "request_id": request["request_id"], "ok": True, "data": data}) + "\n").encode("utf-8"))

            worker = threading.Thread(target=serve)
            worker.start()
            client = CixaClient(socket_path, token_path)
            self.assertEqual(client.get_status(), {"principal": "agent"})
            self.assertEqual(client.list_transactions("intent_cursor", 50)["has_more"], False)
            self.assertEqual(operations[1], {"type": "list_transactions_page", "cursor": "intent_cursor", "limit": 50})
            worker.join(timeout=2)
            listener.close()


if __name__ == "__main__":
    unittest.main()
