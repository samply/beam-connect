
try:
    import requests
except ImportError:
    import os
    os.system("python3 -m pip install requests")
    import requests

import unittest

class TestConnect(unittest.TestCase):

    def test_normal_request(self):
        res = request_connect("http://httpbin.org/anything")
        self.assertEqual(res.status_code, 200, "Could not make normal request via connect")
    
    def test_json_body(self):
        json = {
            "foo": "bar",
            "asdf": 3,
            "baz": [{}, 2]
        }
        res = request_connect("http://httpbin.org/anything", json=json)
        self.assertEqual(res.status_code, 200, "Could not make normal request via connect")   
        self.assertEqual(res.json().get("json"), json, "Json did not match")


def request_connect(url: str, json = {}, app_id: str = "app1.proxy1.broker", app_secret: str = "App1Secret", proxy_url: str = "http://localhost:8062") -> requests.Response:
    proxies = {
        "http": proxy_url
    }
    headers = {
        "Proxy-Authorization": f"ApiKey {app_id} {app_secret}",
        "Accept": "application/json"
    }
    return requests.get(url, json=json, proxies=proxies, headers=headers)


def main():
    unittest.main()

if __name__ == "__main__":
    main()
