PYTHON ?= python3.12
VENV_PYTHON ?= .venv/bin/python

.PHONY: setup test
setup:
	$(PYTHON) -m venv .venv
	$(VENV_PYTHON) -m pip install -r requirements.txt

test:
	$(VENV_PYTHON) -m unittest discover -s tests -p 'test_*.py' -q
