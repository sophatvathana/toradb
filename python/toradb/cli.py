import sys

def main():
    print("toradb cli:", " ".join(sys.argv[1:]) or "(no args)")

if __name__ == "__main__":
    main()
